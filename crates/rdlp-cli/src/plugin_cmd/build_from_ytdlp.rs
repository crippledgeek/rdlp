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
    let valid_urls = extract_valid_urls(&source);
    if valid_urls.is_empty() {
        bail!(
            "could not find any `_VALID_URL` declaration in plugin source — \
             at least one `class FooIE(InfoExtractor): _VALID_URL = r'...'` \
             must exist"
        );
    }
    let matches = valid_urls_to_match_patterns(&valid_urls);
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

/// Find every `_VALID_URL = r'...'` (or `r'''...'''` / `r"""..."""`)
/// declaration in `source`. Returns one entry per concrete IE class.
///
/// Triple-quoted regexes are required for SVT — yt-dlp's `(?x)` verbose
/// mode is line-broken across many physical lines and the single-quote
/// form (`[^'"]+`) cannot capture that. Triple-quote support is a Slice-2
/// requirement, NOT a future enhancement.
///
/// Filters out matches that occur INSIDE a docstring or other
/// triple-quoted string literal. Detection is heuristic: count
/// occurrences of `"""` and `'''` before each candidate match position;
/// if either count is odd, we're inside an unclosed triple-quote (a
/// docstring) and skip. This handles yt-dlp's pattern of putting
/// `_VALID_URL = r'...'` examples inside class/module docstrings.
fn extract_valid_urls(source: &str) -> Vec<String> {
    let triple =
        Regex::new(r#"(?ms)^\s*_VALID_URL\s*=\s*r?(?:'''([\s\S]*?)'''|"""([\s\S]*?)""")"#).unwrap();
    let single = Regex::new(r#"(?m)^\s*_VALID_URL\s*=\s*r?['"]([^'"\n]+)['"]"#).unwrap();

    let mut out: Vec<String> = Vec::new();
    let mut consumed_ranges: Vec<(usize, usize)> = Vec::new();

    for cap in triple.captures_iter(source) {
        let m = cap.get(0).unwrap();
        if is_inside_triple_quote(source, m.start()) {
            // Triple-quoted `_VALID_URL` example inside a docstring.
            // Mark the range as consumed so the single-quote pass
            // doesn't pick up a sub-fragment, but DON'T add to output.
            consumed_ranges.push((m.start(), m.end()));
            continue;
        }
        consumed_ranges.push((m.start(), m.end()));
        let body = cap
            .get(1)
            .or_else(|| cap.get(2))
            .map(|g| g.as_str().to_string())
            .unwrap_or_default();
        out.push(body);
    }
    for cap in single.captures_iter(source) {
        let m = cap.get(0).unwrap();
        // Skip captures inside a triple-quoted range we already saw.
        if consumed_ranges
            .iter()
            .any(|&(s, e)| m.start() >= s && m.start() < e)
        {
            continue;
        }
        if is_inside_triple_quote(source, m.start()) {
            continue;
        }
        out.push(cap[1].to_string());
    }
    out
}

/// Returns true when `position` falls inside an unclosed triple-quoted
/// string literal. Counts `"""` and `'''` occurrences in the prefix; an
/// odd count of either means we're inside a still-open string.
///
/// Heuristic: ignores the case of mixed nested triple-quote chars
/// (e.g. `'''...""".....'''` would count `"""` as 1). yt-dlp source
/// files don't exercise that pattern; if a real plugin does, the
/// author can hand-edit the manifest.
fn is_inside_triple_quote(source: &str, position: usize) -> bool {
    let prefix = &source[..position];
    let triple_double = prefix.matches("\"\"\"").count();
    let triple_single = prefix.matches("'''").count();
    triple_double % 2 == 1 || triple_single % 2 == 1
}

/// Convert a slice of yt-dlp `_VALID_URL` regex strings to Chrome-style
/// match patterns parseable by `rdlp_plugin::dispatch::MatchPattern::parse`.
///
/// Multi-class plugins (e.g. SVT with Play/Series/Page IEs in one file)
/// produce N regexes; this fn unions their host-prefix patterns and
/// dedupes so the manifest's `matches=[...]` doesn't repeat itself when
/// every class shares the same host.
///
/// `MatchPattern` only accepts:
/// - scheme: http | https | * | file
/// - host: * | *.example.com | example.com (no regex chars)
/// - path: anything after `/`
fn valid_urls_to_match_patterns(regexes: &[String]) -> Vec<String> {
    if regexes.is_empty() {
        return vec!["*://*/*".to_string()];
    }
    // Capture host between the scheme and the first `/`. Handle the
    // optional `(?:www\.)?` prefix yt-dlp uses pervasively.
    let with_www = Regex::new(
        r"https\??(?:s\?)?://(?:\(\?:www\\\.\)\?)([a-zA-Z0-9-]+(?:\\?\.[a-zA-Z0-9-]+)+)",
    )
    .unwrap();
    let bare = Regex::new(r"https\??(?:s\?)?://([a-zA-Z0-9-]+(?:\\?\.[a-zA-Z0-9-]+)+)").unwrap();

    let mut out: Vec<String> = Vec::new();
    let mut any_extracted = false;
    for regex in regexes {
        let extracted = if let Some(c) = with_www.captures(regex) {
            let host = c[1].replace(r"\.", ".");
            any_extracted = true;
            vec![format!("https://*.{host}/*"), format!("https://{host}/*")]
        } else if let Some(c) = bare.captures(regex) {
            let host = c[1].replace(r"\.", ".");
            any_extracted = true;
            vec![format!("https://{host}/*")]
        } else {
            // This particular regex is unparseable; skip it. Other
            // regexes in the slice may still extract — only fall back
            // to the wildcard when EVERY regex fails.
            vec![]
        };
        for p in extracted {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    if !any_extracted {
        return vec!["*://*/*".to_string()];
    }
    out
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

    // Slice-2.5: stage the user's plugin into a fake yt_dlp/ package
    // so upstream relative imports (`from .common import InfoExtractor`,
    // `from ..utils import ...`) resolve unchanged.
    let yt_dlp_root = build_dir.join("yt_dlp");
    std::fs::create_dir_all(yt_dlp_root.join("extractor"))?;
    std::fs::create_dir_all(yt_dlp_root.join("utils"))?;

    std::fs::write(yt_dlp_root.join("__init__.py"), YT_DLP_INIT_PY)?;
    std::fs::write(yt_dlp_root.join("extractor/__init__.py"), b"")?;
    std::fs::write(yt_dlp_root.join("extractor/common.py"), EXTRACTOR_COMMON_PY)?;
    std::fs::write(yt_dlp_root.join("utils/__init__.py"), UTILS_INIT_PY)?;
    std::fs::write(yt_dlp_root.join("utils/_utils.py"), UTILS_HELPERS_PY)?;
    std::fs::write(yt_dlp_root.join("utils/traversal.py"), UTILS_TRAVERSAL_PY)?;

    let stem = plugin_py
        .file_stem()
        .and_then(|s| s.to_str())
        .context("invalid plugin filename")?;
    std::fs::copy(
        plugin_py,
        yt_dlp_root.join("extractor").join(format!("{stem}.py")),
    )?;

    // _entry.py — auto-generated wrapper; substitute plugin module name.
    let entry_body = ENTRY_TEMPLATE.replace("{{PLUGIN_MODULE}}", stem);
    std::fs::write(build_dir.join("_entry.py"), entry_body)?;

    Ok(())
}

const YT_DLP_INIT_PY: &str = "\"\"\"rdlp shim — fake yt_dlp package staged by \
build-from-ytdlp.\"\"\"\n__version__ = \"rdlp-shim/0.2\"\n";

const EXTRACTOR_COMMON_PY: &str = "from rdlp_ytdlp_compat import InfoExtractor  # noqa: F401\n";

const UTILS_INIT_PY: &str = "\
from .traversal import *  # noqa: F401, F403
from ._utils import *     # noqa: F401, F403
";

/// `yt_dlp/utils/_utils.py` — re-exports the helpers that live in
/// `rdlp_ytdlp_compat._utils` and `rdlp_ytdlp_compat.info_extractor`.
///
/// Module mapping (verified against the shim source):
/// - `rdlp_ytdlp_compat._utils`: `clean_html`, `determine_ext`, `dict_get`,
///   `format_field`, `merge_dicts`, `parse_duration`, `sanitize_filename`,
///   `sanitize_path`, `str_to_int`, `unified_strdate`, `url_or_none`,
///   `variadic`
/// - `rdlp_ytdlp_compat.info_extractor`: `int_or_none`, `try_get`,
///   `unified_timestamp`, `urljoin`
const UTILS_HELPERS_PY: &str = "\
from rdlp_ytdlp_compat._utils import (  # noqa: F401
    clean_html, determine_ext, dict_get, format_field,
    merge_dicts, parse_duration, sanitize_filename, sanitize_path,
    str_or_none, str_to_int, unified_strdate, url_or_none, variadic,
)
from rdlp_ytdlp_compat.info_extractor import (  # noqa: F401
    int_or_none, try_get, unified_timestamp, urljoin,
)
from rdlp_ytdlp_compat._errors import (  # noqa: F401
    ExtractorError, GeoRestrictedError, RegexNotFoundError,
    UnsupportedError, UserNotLive,
)
";

/// `yt_dlp/utils/traversal.py` — re-exports traversal helpers.
///
/// Module mapping (verified against the shim source):
/// - `rdlp_ytdlp_compat.info_extractor`: `traverse_obj`
/// - `rdlp_ytdlp_compat._utils`: `dict_get`, `require`
const UTILS_TRAVERSAL_PY: &str = "\
from rdlp_ytdlp_compat.info_extractor import (  # noqa: F401
    traverse_obj,
)
from rdlp_ytdlp_compat._utils import (  # noqa: F401
    dict_get, require,
)
";

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
/// 4. Multi-class plugin support (Slice 2): `_entry.py` walks every
///    concrete `InfoExtractor` subclass in `user_plugin` at extract time
///    and dispatches by `cls.suitable(url)`. SVT-style siblings
///    (SVTPlayIE / SVTSeriesIE / SVTPageIE) ship in one .py and the
///    `suitable()` overrides decide which class claims a given URL.
///    Discovery + dispatch live in `rdlp_ytdlp_compat._dispatch` so they
///    are unit-testable in plain CPython.
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
# Plugin source is staged at yt_dlp/extractor/{{PLUGIN_MODULE}}.py
# (Slice-2.5 fake-package layout for byte-identical drop-in).
from yt_dlp.extractor.{{PLUGIN_MODULE}} import *  # noqa: F401,F403

import yt_dlp.extractor.{{PLUGIN_MODULE}} as user_plugin
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
from rdlp_ytdlp_compat._dispatch import (
    DispatchError as _DispatchError,
    discover_ie_classes as _discover_ie_classes,
    dispatch_url as _dispatch_url,
)


# Discover all concrete InfoExtractor subclasses at module load. Exactly
# zero candidates = plugin-authoring error (raised loudly at first call
# below). Multi-class plugins (e.g. SVT with Play/Series/Page IEs in one
# file) keep all classes — extract-time dispatch picks the right one
# per URL via cls.suitable().
try:
    _IE_CLASSES = _discover_ie_classes(user_plugin)
except _DispatchError as _e:
    # Surface at extract() / metadata() rather than here — componentize-py
    # init errors crash the whole instance.
    _IE_CLASSES = []
    _DISCOVERY_ERROR = _e
else:
    _DISCOVERY_ERROR = None

# Pre-instantiate one IE per class. Slice-1 used a single `_IE` instance;
# Slice-2 keeps a dict so dispatch can route URLs to the right object.
_IE_INSTANCES = {cls: cls() for cls in _IE_CLASSES}


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
    # Reuse `_format_payload` so the legacy `cause` attribute (yt-dlp's
    # own convention via `ExtractorError(msg, cause=e)`) is also
    # flattened, not just `__cause__`. Earlier this branch duplicated
    # the cause-flattening inline and dropped the legacy attr — found
    # in code review as a debug-info regression for unknown exception
    # types.
    return ExtractError_Internal(_format_payload(e))


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
        if _DISCOVERY_ERROR is not None:
            raise Err(ExtractError_Internal(str(_DISCOVERY_ERROR)))
        # When multiple IE classes ship in one plugin, the manifest's
        # plugin name is determined by the `.py` filename (kebab-cased
        # at build time). For metadata reporting we use the first class
        # purely for the `url_regex` hint shown in `rdlp plugin info`;
        # actual matching still walks every class via `suitable()`.
        primary = _IE_CLASSES[0]
        return PluginInfo(
            name=primary.__name__.lower(),
            version="0.1.0",
            wit_version="0.1.0",
            matches=[],  # populated from manifest at install time
            url_regex=getattr(primary, "_VALID_URL", None),
            priority=150,
            claims_override=[],
            supports_search=False,
        )

    def extract(self, url: str) -> InfoDict:
        if _DISCOVERY_ERROR is not None:
            raise Err(ExtractError_Internal(str(_DISCOVERY_ERROR)))
        # Walk every IE class in the plugin and pick the first whose
        # `suitable(url)` is True. Sibling-override classes (e.g.
        # SVTSeriesIE.suitable yields when SVTPlayIE.suitable matches)
        # are honoured because dispatch_url respects each class's own
        # suitable() implementation.
        ie_class = _dispatch_url(_IE_CLASSES, url)
        if ie_class is None:
            raise Err(ExtractError_UnsupportedUrl(url))
        ie = _IE_INSTANCES[ie_class]
        # All extraction logic — including _dict_to_info_dict's _opt_str
        # type-check on each format field — runs INSIDE the try. Otherwise
        # _opt_str's _ExtractorError on a bad format dict would propagate
        # past the variant dispatcher as an uncaught Python exception,
        # which componentize-py surfaces as a wasm trap (instance killed,
        # epoch fuel lost) instead of the graceful WIT variant.
        try:
            d = ie._real_extract(url)
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


def _opt_uint(v, where=""):
    """Coerce optional unsigned-int fields to Python int. yt-dlp helpers
    (`parse_duration`, `int_or_none`) often return floats — passing a
    float to a WIT `option<u32>` / `option<u64>` field crashes the
    componentize-py canonical-ABI marshaller (`ToCanonU32`). Round-to-int
    here so plugin authors don't have to remember which fields require
    integer values; bool guarded explicitly because Python `bool` is an
    `int` subclass and silently coercing `True`→`1` is surprising."""
    if v is None:
        return None
    if isinstance(v, bool):
        locus = f" at {where}" if where else ""
        raise _ExtractorError(
            f"[validate] info-dict field{locus} is bool, "
            f"expected number or None",
            expected=False,
        )
    if isinstance(v, int):
        return max(0, v)
    if isinstance(v, float):
        return max(0, int(round(v)))
    locus = f" at {where}" if where else ""
    raise _ExtractorError(
        f"[validate] info-dict field{locus} has invalid type: "
        f"expected number/None, got {type(v).__name__}",
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
    # Single-format-via-`url` canonicalisation. yt-dlp extractors
    # frequently return `{'id': ..., 'url': video_url, ...}` with no
    # explicit `formats[]` (xxxymovies, alphaporno, hellporno, many
    # others). Without this synthesis the WIT info-dict ships an empty
    # `formats[]` list and the URL is silently lost on the host side
    # (`adapter.rs::convert_info_dict` consumes it as `webpage_url` only).
    # Mirrors yt-dlp's internal `_check_formats` canonicalisation.
    if not formats and isinstance(d.get("url"), str) and d["url"]:
        formats.append(Format(
            format_id=_opt_str(d.get("format_id"), "format_id") or "0",
            url=d["url"],
            ext=_opt_str(d.get("ext"), "ext") or "mp4",
            protocol=_opt_str(d.get("protocol"), "protocol") or "https",
            width=d.get("width"), height=d.get("height"), fps=d.get("fps"),
            tbr=d.get("tbr"), vbr=d.get("vbr"), abr=d.get("abr"),
            vcodec=_opt_str(d.get("vcodec"), "vcodec"),
            acodec=_opt_str(d.get("acodec"), "acodec"),
            container=_opt_str(d.get("container"), "container"),
            filesize=d.get("filesize"),
            format_note=_opt_str(d.get("format_note"), "format_note"),
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
        duration=_opt_uint(d.get("duration"), "duration"),
        view_count=_opt_uint(d.get("view_count"), "view_count"),
        like_count=_opt_uint(d.get("like_count"), "like_count"),
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
         # componentize-py-pin@0.17.2 emits IMPORTS for every interface in\n\
         # the WIT world regardless of which the plugin actually uses, so\n\
         # the manifest MUST declare all six host capabilities or the host\n\
         # linker rejects the wasm at instantiation time. Capability-gating\n\
         # still happens at runtime via populate_capability_contexts: a\n\
         # capability declared here but not granted by the host returns\n\
         # \"denied\" when the plugin actually calls it. Hand-edit this list\n\
         # down only if the plugin demonstrably never imports a capability.\n\
         capabilities = [\"fetch\", \"cookie-jar\", \"js-eval\", \"html-select\", \"log\", \"store-kv\"]\n\
         \n\
         # PLACEHOLDER — run `rdlp plugin sign {name}` to populate.\n\
         [signature]\n\
         type = \"ed25519\"\n\
         pubkey = \"PLACEHOLDER_PUBKEY\"\n\
         signature = \"PLACEHOLDER_SIGNATURE\"\n"
    );
    std::fs::write(path, body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_url_to_match_emits_chrome_style_patterns() {
        let patterns = valid_urls_to_match_patterns(&[
            r"https?://(?:www\.)?pornhub\.com/view_video\.php\?viewkey=(?P<id>[^&]+)".to_string(),
        ]);
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
        let patterns =
            valid_urls_to_match_patterns(&[r"https?://example\.com/(?P<id>\d+)".to_string()]);
        assert_eq!(patterns, vec!["https://example.com/*".to_string()]);
        rdlp_plugin::dispatch::MatchPattern::parse(&patterns[0]).unwrap();
    }

    #[test]
    fn valid_url_unparseable_falls_back_to_wildcard() {
        let patterns =
            valid_urls_to_match_patterns(&[r"some-weird-regex-without-host".to_string()]);
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
    fn extract_valid_urls_finds_single_pattern() {
        let src = "\nclass Foo:\n    _VALID_URL = r'https?://example\\.com/(?P<id>\\d+)'\n";
        assert_eq!(
            extract_valid_urls(src),
            vec![r"https?://example\.com/(?P<id>\d+)".to_string()],
        );
    }

    #[test]
    fn extract_valid_urls_finds_multiple_classes() {
        // SVT-like file: 3 concrete IE classes each with their own
        // `_VALID_URL`. All three MUST be captured so the manifest's
        // `matches=[...]` covers every class.
        let src = "\
class APlayIE(Base):
    _VALID_URL = r'https?://a\\.example/play/(?P<id>\\w+)'

class ASeriesIE(Base):
    _VALID_URL = r'https?://a\\.example/series/(?P<id>\\w+)'

class APageIE(Base):
    _VALID_URL = r'https?://a\\.example/page/(?P<id>\\w+)'
";
        let urls = extract_valid_urls(src);
        assert_eq!(urls.len(), 3, "expected 3 _VALID_URL captures");
        assert!(urls.iter().any(|u| u.contains("/play/")));
        assert!(urls.iter().any(|u| u.contains("/series/")));
        assert!(urls.iter().any(|u| u.contains("/page/")));
    }

    #[test]
    fn extract_valid_urls_handles_triple_quoted() {
        // SVT uses r'''...''' for verbose regex. Single-line capture
        // would miss this — test triple-quote support explicitly.
        let src = r#"
class SVTPlayIE(SVTBaseIE):
    _VALID_URL = r'''(?x)
                    (?:
                        svt:|
                        https?://(?:www\.)?svt\.se/foo/
                    )
                    (?P<id>[^/?#&]+)
                    '''
"#;
        let urls = extract_valid_urls(src);
        assert_eq!(urls.len(), 1);
        assert!(urls[0].contains("svt\\.se"), "got: {:?}", urls[0]);
    }

    #[test]
    fn extract_valid_urls_skips_docstring_examples() {
        // A `_VALID_URL = r'...'` literal appearing inside a docstring
        // (or any triple-quoted string that ISN'T itself the assignment)
        // MUST NOT be captured. Otherwise the manifest's `matches=[...]`
        // gets polluted with example URLs that don't reflect any real
        // class. yt-dlp's own extractor docstrings sometimes show such
        // examples — this is real-world risk.
        let src = r#"
class FooIE:
    """Documents the IE.

    Example:
        _VALID_URL = r'https?://docstring-example\.com/(?P<id>\w+)'
    """
    _VALID_URL = r'https?://real-foo\.com/(?P<id>\w+)'
"#;
        let urls = extract_valid_urls(src);
        // Exactly one match — the real assignment. The docstring
        // example must be skipped.
        assert_eq!(urls.len(), 1, "got {urls:?}");
        assert!(urls[0].contains("real-foo"), "got {urls:?}");
    }

    #[test]
    fn extract_valid_urls_skips_single_quote_docstring_example() {
        // Same scenario, single-quoted docstring.
        let src = r#"
class FooIE:
    '''Single-quoted docstring with example:
        _VALID_URL = r'https?://docstring\.example/(?P<id>\w+)'
    '''
    _VALID_URL = r'https?://real\.example/(?P<id>\w+)'
"#;
        let urls = extract_valid_urls(src);
        assert_eq!(urls.len(), 1, "got {urls:?}");
        assert!(urls[0].contains(r"real\.example"), "got {urls:?}");
    }

    #[test]
    fn extract_valid_urls_returns_empty_when_none_present() {
        // Plain helper module without any `_VALID_URL` declaration —
        // returns empty Vec rather than an error sentinel.
        let src = "def helper(): return 42\n";
        assert!(extract_valid_urls(src).is_empty());
    }

    #[test]
    fn valid_url_to_match_patterns_unions_multiple_hosts() {
        // Three IEs against the same host produce one deduped match
        // pattern, not three duplicates.
        let urls = vec![
            r"https?://(?:www\.)?example\.com/play/(?P<id>\w+)".to_string(),
            r"https?://(?:www\.)?example\.com/series/(?P<id>\w+)".to_string(),
            r"https?://(?:www\.)?example\.com/page/(?P<id>\w+)".to_string(),
        ];
        let patterns = valid_urls_to_match_patterns(&urls);
        // Deduped — both *.example.com and example.com appear once each
        // even though three input URLs share the same host shape.
        assert!(patterns.contains(&"https://*.example.com/*".to_string()));
        assert!(patterns.contains(&"https://example.com/*".to_string()));
        assert_eq!(patterns.len(), 2);
    }

    #[test]
    fn valid_url_to_match_patterns_handles_distinct_hosts() {
        let urls = vec![
            r"https?://alpha\.example/(?P<id>\w+)".to_string(),
            r"https?://beta\.example/(?P<id>\w+)".to_string(),
        ];
        let patterns = valid_urls_to_match_patterns(&urls);
        assert!(patterns.contains(&"https://alpha.example/*".to_string()));
        assert!(patterns.contains(&"https://beta.example/*".to_string()));
        assert_eq!(patterns.len(), 2);
    }

    #[test]
    fn valid_url_to_match_patterns_empty_input_returns_wildcard() {
        // No `_VALID_URL` found anywhere — fall back to wildcard so the
        // author gets the same warning path as before, not a panic.
        let patterns = valid_urls_to_match_patterns(&[]);
        assert_eq!(patterns, vec!["*://*/*".to_string()]);
    }

    #[test]
    fn stage_build_dir_creates_fake_yt_dlp_package() {
        let tmp = tempfile::tempdir().unwrap();
        let plugin = tmp.path().join("foo.py");
        std::fs::write(&plugin, "from rdlp_ytdlp_compat import InfoExtractor\n").unwrap();
        let workspace = locate_workspace_root().unwrap();
        let wit = workspace.join("crates/rdlp-plugin/wit");
        let build = tmp.path().join("build");
        std::fs::create_dir(&build).unwrap();
        stage_build_dir(&build, &plugin, &workspace, &wit).unwrap();
        // The fake yt-dlp tree must exist.
        assert!(build.join("yt_dlp/__init__.py").exists());
        assert!(build.join("yt_dlp/extractor/__init__.py").exists());
        assert!(build.join("yt_dlp/extractor/common.py").exists());
        assert!(build.join("yt_dlp/extractor/foo.py").exists());
        assert!(build.join("yt_dlp/utils/__init__.py").exists());
        assert!(build.join("yt_dlp/utils/_utils.py").exists());
        assert!(build.join("yt_dlp/utils/traversal.py").exists());
        let plugin_staged = std::fs::read(build.join("yt_dlp/extractor/foo.py")).unwrap();
        let plugin_orig = std::fs::read(&plugin).unwrap();
        assert_eq!(plugin_staged, plugin_orig);
        // The _entry.py placeholder must be substituted with the plugin stem.
        let entry = std::fs::read_to_string(build.join("_entry.py")).unwrap();
        assert!(
            entry.contains("yt_dlp.extractor.foo"),
            "stem not substituted in _entry.py"
        );
        assert!(
            !entry.contains("{{PLUGIN_MODULE}}"),
            "placeholder still present in _entry.py"
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
        // The default capability set MUST cover every interface
        // componentize-py emits in the WIT world (instantiation fails
        // otherwise — see the capabilities-line doc-comment in
        // `write_manifest`).
        assert_eq!(
            manifest.capabilities,
            vec![
                "fetch",
                "cookie-jar",
                "js-eval",
                "html-select",
                "log",
                "store-kv"
            ],
        );
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
