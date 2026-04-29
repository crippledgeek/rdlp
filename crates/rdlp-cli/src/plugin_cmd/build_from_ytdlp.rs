//! `rdlp plugin build-from-ytdlp <plugin.py>` — wraps componentize-py-pin@0.17.2
//! to bundle a yt-dlp-style Python extractor + the `rdlp_ytdlp_compat` shim
//! into a Component Model `.wasm` plus a `plugin.toml.template` manifest.

// CLI command — sync I/O is acceptable; matches the rest of plugin_cmd.
#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use regex::Regex;

/// Run the `rdlp plugin build-from-ytdlp` command — invokes componentize-py
/// to produce `<output_dir>/<name>/plugin.wasm` + `plugin.toml.template`.
pub async fn run(plugin_py: PathBuf, output_dir: Option<PathBuf>) -> Result<()> {
    let py_path = plugin_py
        .canonicalize()
        .with_context(|| format!("input not found: {}", plugin_py.display()))?;
    let output_dir =
        output_dir.unwrap_or_else(|| py_path.parent().unwrap_or(Path::new(".")).to_path_buf());
    let raw_stem = py_path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("invalid plugin filename")?
        .to_string();
    // Python source filenames conventionally use snake_case (`simple_html.py`)
    // because they have to be importable Python identifiers. Plugin manifest
    // names are kebab-case for filesystem/sled-namespace safety. Translate at
    // this boundary: `simple_html` → `simple-html`. Lowercasing handles any
    // PascalCase quirks. Surfacing the translation in stderr lets the author
    // catch surprises before signing.
    let stem = raw_stem.to_ascii_lowercase().replace('_', "-");
    if stem != raw_stem {
        eprintln!(
            "Note: plugin filename '{raw_stem}' normalised to plugin name '{stem}' \
             (manifest names are kebab-case)."
        );
    }
    // The stem becomes the plugin name in the manifest (used as a TOML string,
    // a filesystem subdir, and a sled-namespace key). Enforce the same shape
    // the loader will demand at install time, so authors get a clear error
    // here instead of an opaque manifest-parse failure later.
    rdlp_plugin::manifest::validate_plugin_name(&stem)
        .with_context(|| format!("plugin filename '{raw_stem}' not a valid plugin name"))?;

    let source = std::fs::read_to_string(&py_path)?;
    let valid_url =
        extract_valid_url(&source).context("could not find _VALID_URL in plugin source")?;
    let matches = valid_url_to_match_patterns(&valid_url);
    // The fallback pattern `*://*/*` matches the entire internet at priority
    // 150, shadowing every built-in extractor that doesn't explicitly opt-in
    // to override-claiming. Authors who hit it on a complex `_VALID_URL` (e.g.
    // alternation in TLDs, non-trivial subdomain regex) should hand-edit the
    // generated manifest before signing. Surface this loudly so it isn't
    // silent.
    if matches.iter().any(|p| p == "*://*/*") {
        eprintln!(
            "WARNING: could not extract a literal hostname from `_VALID_URL`. \
             Generated manifest uses the over-broad `*://*/*` match pattern, \
             which intercepts every URL. Hand-edit \
             `{stem}/plugin.toml.template` `matches = [...]` to your site \
             before signing."
        );
    }

    let workspace_root = locate_workspace_root()?;
    let venv = workspace_root.join("tools/ytdlp-compat/.venv");
    if !venv.exists() {
        bail!(
            "tools/ytdlp-compat/.venv not found. Run:\n\
             cd tools/ytdlp-compat && python3 -m venv .venv && \\\n\
             .venv/bin/pip install -r requirements-dev.txt"
        );
    }
    let wit_dir = workspace_root.join("crates/rdlp-plugin/wit");

    // Plugin output dir: <output_dir>/<name>/{plugin.wasm, plugin.toml.template}
    let plugin_subdir = output_dir.join(&stem);
    std::fs::create_dir_all(&plugin_subdir).context("create plugin output subdir")?;

    let build_dir = tempfile::tempdir().context("create build dir")?;
    stage_build_dir(build_dir.path(), &py_path, &workspace_root, &wit_dir)?;

    let componentize_py = venv.join("bin/componentize-py");
    let world_name = "extractor-plugin";

    // componentize-py-pin@0.17.2: dirty bindings dir errors with EEXIST. Clean first.
    let bindings_dir = build_dir.path().join("extractor_plugin");
    if bindings_dir.exists() {
        std::fs::remove_dir_all(&bindings_dir).ok();
    }

    let bindings_status = Command::new(&componentize_py)
        .args([
            "-d",
            "wit",
            "-w",
            world_name,
            "--world-module",
            "extractor_plugin",
            "bindings",
            ".",
        ])
        .current_dir(build_dir.path())
        .status()
        .context("invoke componentize-py bindings")?;
    if !bindings_status.success() {
        bail!("componentize-py bindings failed");
    }

    let wasm_out = plugin_subdir.join("plugin.wasm");
    // `--stub-wasi` is a flag on the `componentize` subcommand (not global) —
    // host doesn't link WASI 0.2 imports (Phase-1 limitation).
    let componentize_status = Command::new(&componentize_py)
        .args([
            "-d",
            "wit",
            "-w",
            world_name,
            "--world-module",
            "extractor_plugin",
            "componentize",
            "--stub-wasi",
            "_entry",
        ])
        .arg("-o")
        .arg(&wasm_out)
        .current_dir(build_dir.path())
        .status()
        .context("invoke componentize-py componentize")?;
    if !componentize_status.success() {
        bail!("componentize-py componentize failed");
    }

    // Manifest emitted as `.template` — production users run `rdlp plugin sign`
    // to fill in the [signature] block before installing.
    let toml_out = plugin_subdir.join("plugin.toml.template");
    write_manifest(&toml_out, &stem, &matches)?;

    eprintln!(
        "Built: {} ({} bytes)",
        wasm_out.display(),
        std::fs::metadata(&wasm_out)?.len()
    );
    eprintln!("Manifest: {}", toml_out.display());
    eprintln!("Sign with: rdlp plugin sign {stem}");
    Ok(())
}

fn extract_valid_url(source: &str) -> Option<String> {
    // Match: _VALID_URL = r'...' / "..."  (single or double quotes, raw or not).
    let re = Regex::new(r#"(?m)^\s*_VALID_URL\s*=\s*r?['"]([^'"]+)['"]"#).unwrap();
    re.captures(source).map(|c| c[1].to_string())
}

/// Convert a yt-dlp regex `_VALID_URL` to Chrome-style match patterns
/// parseable by `rdlp_plugin::dispatch::MatchPattern::parse`.
///
/// `MatchPattern` only accepts:
/// - scheme: http | https | * | file
/// - host: * | *.example.com | example.com (no regex chars)
/// - path: anything after `/`
fn valid_url_to_match_patterns(regex: &str) -> Vec<String> {
    // Capture host between the scheme and the first `/`.
    // Handle optional `(?:www\.)?` prefix.
    let with_www = Regex::new(
        r"^https\??(?:s\?)?://(?:\(\?:www\\\.\)\?)([a-zA-Z0-9-]+(?:\\?\.[a-zA-Z0-9-]+)+)",
    )
    .unwrap();
    let bare = Regex::new(r"^https\??(?:s\?)?://([a-zA-Z0-9-]+(?:\\?\.[a-zA-Z0-9-]+)+)").unwrap();

    if let Some(c) = with_www.captures(regex) {
        let host = c[1].replace(r"\.", ".");
        return vec![format!("https://*.{host}/*"), format!("https://{host}/*")];
    }
    if let Some(c) = bare.captures(regex) {
        let host = c[1].replace(r"\.", ".");
        return vec![format!("https://{host}/*")];
    }
    // Fallback: over-broad. Authors should hand-edit before publishing.
    vec!["*://*/*".to_string()]
}

fn locate_workspace_root() -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .context("invoke `cargo locate-project` (is cargo on PATH?)")?;
    if !output.status.success() {
        bail!(
            "`cargo locate-project --workspace` exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        );
    }
    let path = String::from_utf8(output.stdout)
        .context("cargo locate-project produced non-UTF-8 output")?
        .trim()
        .to_string();
    if path.is_empty() {
        bail!("cargo locate-project produced empty output (run from inside a workspace)");
    }
    Ok(PathBuf::from(path)
        .parent()
        .context("workspace root has no parent")?
        .to_path_buf())
}

fn stage_build_dir(
    build_dir: &Path,
    plugin_py: &Path,
    workspace_root: &Path,
    wit_src: &Path,
) -> Result<()> {
    // Copy WIT files
    let wit_dst = build_dir.join("wit");
    std::fs::create_dir_all(&wit_dst)?;
    for entry in std::fs::read_dir(wit_src)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) == Some("wit") {
            std::fs::copy(entry.path(), wit_dst.join(entry.file_name()))?;
        }
    }

    // Copy rdlp_ytdlp_compat package
    let compat_pkg = workspace_root.join("tools/ytdlp-compat/rdlp_ytdlp_compat");
    let compat_dst = build_dir.join("rdlp_ytdlp_compat");
    copy_dir_all(&compat_pkg, &compat_dst)?;

    // Copy user plugin
    std::fs::copy(plugin_py, build_dir.join("user_plugin.py"))?;

    // _entry.py — auto-generated wrapper
    std::fs::write(build_dir.join("_entry.py"), ENTRY_TEMPLATE)?;

    Ok(())
}

/// Auto-generated `_entry.py` body — wraps the user's yt-dlp-style
/// `InfoExtractor` subclass and adapts it to the WIT `extractor-plugin`
/// world. Load-bearing invariants (failing any will break dispatch):
///
/// 1. The class implementing `metadata`/`extract`/`search` MUST be named
///    `ExtractorPlugin` because componentize-py-pin@0.17.2 looks up a concrete
///    class whose name matches `--world-module` in PascalCase. Renaming it
///    produces `Can't instantiate abstract class ExtractorPlugin` at load.
/// 2. All imports stay at module top level (componentize-py issue #23 —
///    lazy `__import__()` silently fails inside the bundled CPython).
/// 3. Errors raise via `Err(<variant>)`, NOT return, because the WIT
///    Protocol method signature is the `Ok` payload only — see
///    `extractor_plugin/types.py::Err` (a frozen-dataclass Exception).
/// 4. Multiple `InfoExtractor` subclasses in `user_plugin` are an explicit
///    error: alphabetical-first selection silently dropped sibling
///    extractors before this commit. Per-file plugins must declare exactly
///    one extractor; multi-class sites should ship one .py per extractor.
/// 5. info_dict shape is validated per yt-dlp's documented contract
///    (`yt_dlp/extractor/common.py:107-498` at upstream tag 2026.03.17):
///    `id` and `title` are required strs; either `formats` or `url` must
///    be present.
/// 6. Python exceptions are mapped to WIT variants via a pure `isinstance`
///    ladder — see `_extractor_error_to_variant`. componentize-py 0.17.2
///    only marshals `Err.value` across the WIT boundary (`__cause__` is
///    dropped), so the dispatcher flattens one level of `__cause__` /
///    `cause` (yt-dlp legacy attr) plus `video_id` and `ie` into the
///    payload string at boundary crossing.
const ENTRY_TEMPLATE: &str = r#""""Auto-generated entry point for rdlp plugin build-from-ytdlp.

See the Rust doc-comment on ENTRY_TEMPLATE in build_from_ytdlp.rs for the
load-bearing invariants this file maintains. Editing this file directly is
pointless — it is regenerated on every build-from-ytdlp invocation.
"""
from extractor_plugin import ExtractorPlugin as _ExtractorPluginProtocol
from extractor_plugin.types import Err
from extractor_plugin.imports.types import (
    InfoDict, Format, PluginInfo, SearchPage,
    ExtractError_UnsupportedUrl, ExtractError_NotFound,
    ExtractError_RateLimited, ExtractError_AuthRequired,
    ExtractError_Network, ExtractError_Parse, ExtractError_Cancelled,
    ExtractError_Internal, SearchError_Unsupported,
)

# User plugin imports — must be top-level (componentize-py #23).
from user_plugin import *  # noqa: F401,F403

import user_plugin
from rdlp_ytdlp_compat import (
    InfoExtractor as _CompatInfoExtractor,
    ExtractorError as _ExtractorError,
    UnsupportedError as _UnsupportedError,
    GeoRestrictedError as _GeoRestrictedError,
    UserNotLive as _UserNotLive,
    RegexNotFoundError as _RegexNotFoundError,
    DownloadCancelled as _DownloadCancelled,
    LoginRequiredError as _LoginRequiredError,
    NoFormatsError as _NoFormatsError,
    NotFoundError as _NotFoundError,
    RateLimitedError as _RateLimitedError,
    NetworkError as _NetworkError,
)
from rdlp_ytdlp_compat._host import HostHttpError as _HostHttpError


def _discover_ie_class():
    """Find the user's InfoExtractor subclass. Multiple matches are an error
    (alphabetical-first selection silently drops siblings); zero matches is
    also an error.
    """
    candidates = []
    for _name in dir(user_plugin):
        _v = getattr(user_plugin, _name)
        if (isinstance(_v, type)
                and issubclass(_v, _CompatInfoExtractor)
                and _v is not _CompatInfoExtractor):
            candidates.append(_v)
    if not candidates:
        raise RuntimeError(
            "no InfoExtractor subclass found in plugin — declare "
            "`class FooIE(InfoExtractor):` at module top level"
        )
    if len(candidates) > 1:
        names = ", ".join(c.__name__ for c in candidates)
        raise RuntimeError(
            "multiple InfoExtractor subclasses found in plugin: "
            f"[{names}]. build-from-ytdlp supports exactly one extractor "
            "per .py file; split each into its own file or build separately."
        )
    return candidates[0]


_IE_CLASS = _discover_ie_class()
_IE = _IE_CLASS()


def _format_payload(e):
    """Flatten a Python exception into a WIT-payload string.

    componentize-py 0.17.2 only marshals `Err.value` (the variant payload)
    across the canonical-ABI boundary — `__cause__`, `__context__`,
    `__traceback__`, and `args` are all dropped. So debugging info (cause
    chain, video_id, extractor identity) MUST be flattened into the payload
    string at the dispatch site, otherwise the host sees `"plugin returned
    None"` with zero context.

    Reads BOTH Python's `__cause__` (set by `raise X() from e`, the modern
    PEP-3134 form) AND yt-dlp's legacy `cause` attribute (its own
    convention, set via `ExtractorError(msg, cause=e)`). Walks one level
    deep — deeper chains rarely add signal, and the host stderr captures
    the full traceback via the `log` capability if anyone needs it.
    """
    msg = str(getattr(e, "orig_msg", e) or "")
    # __cause__ first (PEP 3134), fall back to yt-dlp's `cause` attribute.
    cause = getattr(e, "__cause__", None)
    if cause is None:
        cause = getattr(e, "cause", None)
    if cause is not None:
        msg = f"{msg} (cause: {type(cause).__name__}: {cause})"
    vid = getattr(e, "video_id", None)
    if vid:
        msg = f"{msg} [video_id={vid}]"
    ie = getattr(e, "ie", None)
    if ie:
        msg = f"{msg} [ie={ie}]"
    return msg


def _extractor_error_to_variant(e):
    """Map a Python exception to the appropriate WIT extract-error variant.

    Pure `isinstance` ladder — leaf-first (most-specific subclasses checked
    before their parents) so e.g. `LoginRequiredError` (subclass of
    `ExtractorError`) routes to `auth-required` instead of falling through
    to the parent `ExtractorError` arm. Substring matching on message text
    is intentionally absent; relying on it produced false-positive routing
    of real bugs to "expected" variants, masking them from bug reports.

    Variants are listed in WIT-declaration order (see
    `crates/rdlp-plugin/wit/types.wit::extract-error`):

      unsupported-url(string)  — UnsupportedError
      not-found(string)        — UserNotLive, NoFormatsError, NotFoundError
      rate-limited(option<u32>) — RateLimitedError, HTTP 429
      auth-required(string)    — LoginRequiredError, GeoRestrictedError, HTTP 401/403
      network(string)          — NetworkError, HostHttpError (other 4xx/5xx)
      parse(string)            — RegexNotFoundError, bare ExtractorError(expected=True)
      cancelled                — DownloadCancelled
      internal(string)         — ExtractorError(expected=False), unknown exceptions
    """
    # === Typed subclasses — most-specific first =============================
    if isinstance(e, _UnsupportedError):
        return ExtractError_UnsupportedUrl(getattr(e, "url", _format_payload(e)))
    if isinstance(e, _LoginRequiredError):
        return ExtractError_AuthRequired(_format_payload(e))
    if isinstance(e, _GeoRestrictedError):
        # No dedicated geo variant in Slice-1 WIT; auth-required is the
        # closest match (both mean "you can't access this content").
        return ExtractError_AuthRequired(_format_payload(e))
    if isinstance(e, _NoFormatsError):
        return ExtractError_NotFound(_format_payload(e))
    if isinstance(e, _NotFoundError):
        return ExtractError_NotFound(_format_payload(e))
    if isinstance(e, _UserNotLive):
        return ExtractError_NotFound(_format_payload(e))
    if isinstance(e, _RateLimitedError):
        # `retry_after` carried as a typed attribute, NOT parsed from
        # message text — survives the WIT boundary as `option<u32>`.
        return ExtractError_RateLimited(e.retry_after)
    if isinstance(e, _NetworkError):
        return ExtractError_Network(_format_payload(e))
    if isinstance(e, _RegexNotFoundError):
        return ExtractError_Parse(_format_payload(e))
    if isinstance(e, _DownloadCancelled):
        return ExtractError_Cancelled()

    # === Bare ExtractorError parent ========================================
    # Default for expected=True: parse (yt-dlp semantics — "site told us no,
    # not our bug" maps closest to "we couldn't extract", which is parse).
    # Default for expected=False: internal (the extractor likely has a bug).
    if isinstance(e, _ExtractorError):
        msg = _format_payload(e)
        if getattr(e, "expected", False):
            return ExtractError_Parse(msg)
        return ExtractError_Internal(msg)

    # === Typed host HTTP error =============================================
    # _host.HostHttpError carries `.status` as a typed int attribute, so we
    # route by status code without parsing message strings (closes the
    # pre-fix "any RuntimeError starting with 'HTTP ' would mis-dispatch"
    # latent bug).
    if isinstance(e, _HostHttpError):
        status = e.status
        msg = str(e)
        if status == 429:
            return ExtractError_RateLimited(None)
        if status in (401, 403):
            return ExtractError_AuthRequired(msg)
        if status == 404:
            return ExtractError_NotFound(msg)
        if 400 <= status < 600:
            return ExtractError_Network(msg)
        # Status outside 4xx/5xx falling through here means caller passed
        # an `expected_status` that masked it — surface as Internal.
        return ExtractError_Internal(msg)

    # === Catch-all — extractor bug ========================================
    # Flatten Python's __cause__ if present (re-raised exceptions from
    # buggy plugin code).
    msg = str(e)
    cause = getattr(e, "__cause__", None)
    if cause is not None:
        msg = f"{msg} (cause: {type(cause).__name__}: {cause})"
    return ExtractError_Internal(msg)


def _validate_id(d):
    """yt-dlp's info_dict contract requires `id` and `title` as strs and
    either `formats` (list[dict]) OR `url` (str)
    (yt_dlp/extractor/common.py:122-129 @ tag 2026.03.17). Validate at the
    boundary so a buggy plugin returning {"id": None} surfaces a clear
    `ExtractError_Internal` instead of writing the literal string "None"
    into the archive.

    Errors are tagged "[validate]" so a debugger reading host logs can
    distinguish "plugin returned bad shape" from "plugin's _real_extract
    raised". Routes to ExtractError_Internal via the bare-ExtractorError
    arm of _extractor_error_to_variant.
    """
    if not isinstance(d, dict):
        raise _ExtractorError(
            f"[validate] plugin _real_extract returned {type(d).__name__}, "
            f"expected dict",
            expected=False,
        )
    vid = d.get("id")
    if not isinstance(vid, str) or not vid:
        raise _ExtractorError(
            f"[validate] info_dict 'id' must be a non-empty str, "
            f"got {type(vid).__name__}",
            expected=False,
        )
    title = d.get("title")
    if not isinstance(title, str):
        raise _ExtractorError(
            f"[validate] info_dict 'title' must be a str, "
            f"got {type(title).__name__}",
            expected=False,
        )
    formats = d.get("formats")
    url = d.get("url")
    has_formats = isinstance(formats, list) and len(formats) > 0
    has_url = isinstance(url, str) and url
    if not (has_formats or has_url):
        raise _ExtractorError(
            "[validate] info_dict has neither non-empty 'formats' nor 'url'",
            expected=False,
        )


# CRITICAL: class name must be `ExtractorPlugin` (matches --world-module
# PascalCase) for componentize-py-pin@0.17.2 to discover and instantiate it.
class ExtractorPlugin(_ExtractorPluginProtocol):
    def metadata(self) -> PluginInfo:
        return PluginInfo(
            name=_IE_CLASS.__name__.lower(),
            version="0.1.0",
            wit_version="0.1.0",
            matches=[],  # populated from manifest at install time
            url_regex=getattr(_IE_CLASS, "_VALID_URL", None),
            priority=150,
            claims_override=[],
            supports_search=False,
        )

    def extract(self, url: str) -> InfoDict:
        # All extraction logic — including _dict_to_info_dict's _opt_str
        # type-check on each format field — runs INSIDE the try. Otherwise
        # _opt_str's _ExtractorError on a bad format dict would propagate
        # past the variant dispatcher as an uncaught Python exception,
        # which componentize-py surfaces as a wasm trap (instance killed,
        # epoch fuel lost) instead of the graceful WIT variant.
        try:
            d = _IE._real_extract(url)
            _validate_id(d)
            return _dict_to_info_dict(d)
        except Exception as e:
            raise Err(_extractor_error_to_variant(e))

    def search(self, query) -> SearchPage:
        raise Err(SearchError_Unsupported())


def _opt_str(v, where=""):
    """Coerce optional str fields. None → None (not 'None'); int/float are
    converted only if the field's WIT type is `option<string>`. Anything
    else (list, dict) is rejected as a plugin bug.

    `where` is a human-readable location hint ("formats[3].format_id") that
    surfaces in the validation-error message — without it, debugging a
    deeply-nested type mismatch in an info-dict means trial-and-error.
    """
    if v is None:
        return None
    if isinstance(v, str):
        return v
    if isinstance(v, (int, float)):
        return str(v)
    locus = f" at {where}" if where else ""
    raise _ExtractorError(
        f"[validate] info-dict field{locus} has invalid type: "
        f"expected str/None, got {type(v).__name__}",
        expected=False,
    )


def _dict_to_info_dict(d: dict) -> InfoDict:
    # Build formats with index-aware error messages so a malformed
    # `formats[4].format_id` surfaces with that exact path, not just
    # `"info-dict field has invalid type: list"`.
    formats = []
    for i, f in enumerate(d.get("formats") or []):
        if not isinstance(f, dict):
            raise _ExtractorError(
                f"[validate] formats[{i}] must be a dict, "
                f"got {type(f).__name__}",
                expected=False,
            )
        formats.append(Format(
            format_id=_opt_str(f.get("format_id"), f"formats[{i}].format_id") or "",
            url=_opt_str(f.get("url"), f"formats[{i}].url") or "",
            ext=_opt_str(f.get("ext"), f"formats[{i}].ext") or "mp4",
            protocol=_opt_str(f.get("protocol"), f"formats[{i}].protocol") or "https",
            width=f.get("width"), height=f.get("height"), fps=f.get("fps"),
            tbr=f.get("tbr"), vbr=f.get("vbr"), abr=f.get("abr"),
            vcodec=_opt_str(f.get("vcodec"), f"formats[{i}].vcodec"),
            acodec=_opt_str(f.get("acodec"), f"formats[{i}].acodec"),
            container=_opt_str(f.get("container"), f"formats[{i}].container"),
            filesize=f.get("filesize"),
            format_note=_opt_str(f.get("format_note"), f"formats[{i}].format_note"),
        ))
    # Use defensive .get with explicit default ("") — _validate_id already
    # ran and rejected non-str id/title, but keeping the .get makes the
    # coupling resilient to future refactors that might bypass the
    # validator.
    return InfoDict(
        id=d.get("id") or "",
        title=d.get("title") or "",
        url=_opt_str(d.get("url"), "url"),
        formats=formats, subtitles=[],
        thumbnail=_opt_str(d.get("thumbnail"), "thumbnail"),
        description=_opt_str(d.get("description"), "description"),
        uploader=_opt_str(d.get("uploader"), "uploader"),
        uploader_id=_opt_str(d.get("uploader_id"), "uploader_id"),
        upload_date=_opt_str(d.get("upload_date"), "upload_date"),
        duration=d.get("duration"), view_count=d.get("view_count"),
        like_count=d.get("like_count"),
        tags=list(d.get("tags") or []),
        categories=list(d.get("categories") or []),
    )
"#;

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        // Skip __pycache__ but keep __init__.py (must be present for package).
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "__pycache__" {
            continue;
        }
        if src_path.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn write_manifest(path: &Path, name: &str, matches: &[String]) -> Result<()> {
    // Schema verified against `crates/rdlp-plugin-manifest/src/lib.rs::Manifest`.
    // `#[serde(deny_unknown_fields)]` rejects extra keys; do NOT add a [wasm]
    // table or sha256 (signature covers integrity).
    // Capability vocab uses unprefixed names per KNOWN_CAPABILITIES.
    let matches_toml = matches
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let body = format!(
        "name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         wit_version = \"0.1.0\"\n\
         matches = [{matches_toml}]\n\
         priority = 150\n\
         claims_override = []\n\
         supports_search = false\n\
         capabilities = [\"fetch\", \"log\"]\n\
         \n\
         # PLACEHOLDER — run `rdlp plugin sign {name}` to populate.\n\
         [signature]\n\
         type = \"ed25519\"\n\
         pubkey = \"REPLACE_WITH_BASE64_PUBKEY\"\n\
         signature = \"REPLACE_WITH_BASE64_SIGNATURE\"\n"
    );
    std::fs::write(path, body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_url_to_match_emits_chrome_style_patterns() {
        let patterns = valid_url_to_match_patterns(
            r"https?://(?:www\.)?pornhub\.com/view_video\.php\?viewkey=(?P<id>[^&]+)",
        );
        assert!(
            patterns.iter().any(|p| p == "https://*.pornhub.com/*"),
            "expected '*.pornhub.com' pattern, got: {patterns:?}"
        );
        assert!(
            patterns.iter().any(|p| p == "https://pornhub.com/*"),
            "expected 'pornhub.com' pattern, got: {patterns:?}"
        );
        // Round-trip through MatchPattern::parse — every emitted pattern MUST be valid.
        for p in &patterns {
            rdlp_plugin::dispatch::MatchPattern::parse(p)
                .unwrap_or_else(|e| panic!("emitted invalid match pattern {p:?}: {e:?}"));
        }
    }

    #[test]
    fn valid_url_bare_host_no_www_prefix() {
        let patterns = valid_url_to_match_patterns(r"https?://example\.com/(?P<id>\d+)");
        assert_eq!(patterns, vec!["https://example.com/*".to_string()]);
        rdlp_plugin::dispatch::MatchPattern::parse(&patterns[0]).unwrap();
    }

    #[test]
    fn valid_url_unparseable_falls_back_to_wildcard() {
        let patterns = valid_url_to_match_patterns(r"some-weird-regex-without-host");
        assert_eq!(patterns, vec!["*://*/*".to_string()]);
        rdlp_plugin::dispatch::MatchPattern::parse(&patterns[0]).unwrap();
    }

    #[test]
    fn stem_normalisation_underscore_to_hyphen() {
        // build-from-ytdlp normalises Python snake_case filenames to
        // kebab-case plugin names. Pin the contract so the golden corpus
        // (`simple_html.py` etc.) keeps producing valid manifest names.
        let normalised = "simple_html".to_ascii_lowercase().replace('_', "-");
        assert_eq!(normalised, "simple-html");
        rdlp_plugin::manifest::validate_plugin_name(&normalised).unwrap();
    }

    #[test]
    fn stem_normalisation_passes_clean_kebab_through() {
        // Already-kebab names are unchanged.
        let normalised = "my-plugin".to_ascii_lowercase().replace('_', "-");
        assert_eq!(normalised, "my-plugin");
    }

    #[test]
    fn stem_normalisation_lowercases_pascal_case() {
        // PascalCase Python filenames (rare but legal) lowercase cleanly.
        let normalised = "SimplePlugin".to_ascii_lowercase().replace('_', "-");
        assert_eq!(normalised, "simpleplugin");
        rdlp_plugin::manifest::validate_plugin_name(&normalised).unwrap();
    }

    #[test]
    fn extract_valid_url_finds_pattern() {
        let src = "\nclass Foo:\n    _VALID_URL = r'https?://example\\.com/(?P<id>\\d+)'\n";
        assert_eq!(
            extract_valid_url(src),
            Some(r"https?://example\.com/(?P<id>\d+)".to_string())
        );
    }

    #[test]
    fn template_manifest_parses_against_real_schema() {
        // Round-trip the emitted template through the real Manifest parser —
        // catches schema drift automatically (a field rename in
        // rdlp-plugin-manifest::Manifest would fail this test). String-contains
        // assertions remain as belt-and-suspenders for the forbidden [wasm]
        // table and capability-vocab.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plugin.toml.template");
        write_manifest(&path, "test-plugin", &["https://example.com/*".to_string()]).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();

        // 1. Real schema must accept the body (placeholder base64 strings are
        // structurally valid even if cryptographically meaningless until
        // `rdlp plugin sign` runs).
        let manifest = rdlp_plugin_manifest::parse_manifest_str(&body)
            .expect("emitted template must round-trip through parse_manifest_str");
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.matches, vec!["https://example.com/*".to_string()]);
        assert_eq!(manifest.priority, 150);
        assert_eq!(manifest.capabilities, vec!["fetch", "log"]);
        assert!(matches!(
            manifest.signature,
            rdlp_plugin_manifest::Signature::Ed25519 { .. }
        ));

        // 2. String-level invariants — defensive guards against schema regressions
        // that would still parse but ship the wrong shape.
        assert!(
            !body.contains("[wasm]"),
            "template has forbidden [wasm] table"
        );
        assert!(
            !body.contains("\"host:fetch\""),
            "capability vocab must be unprefixed"
        );
    }
}
