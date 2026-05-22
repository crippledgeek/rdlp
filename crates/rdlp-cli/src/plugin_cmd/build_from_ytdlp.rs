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
///
/// # Errors
///
/// Returns an error if the input file cannot be canonicalized, the plugin
/// filename is invalid, `componentize-py` is not found or fails, or the
/// manifest template cannot be written.
#[allow(clippy::too_many_lines)] // sequential build steps; extracting sub-functions would obscure the pipeline
pub async fn run(plugin_py: PathBuf, output_dir: Option<PathBuf>) -> Result<()> {
    let py_path = plugin_py
        .canonicalize()
        .with_context(|| format!("input not found: {}", plugin_py.display()))?;
    let output_dir = output_dir.unwrap_or_else(|| {
        py_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    });
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
    #[allow(clippy::unwrap_used)] // static literal patterns — compile-time valid
    let triple =
        Regex::new(r#"(?ms)^\s*_VALID_URL\s*=\s*r?(?:'''([\s\S]*?)'''|"""([\s\S]*?)""")"#).unwrap();
    #[allow(clippy::unwrap_used)] // static literal pattern — compile-time valid
    let single = Regex::new(r#"(?m)^\s*_VALID_URL\s*=\s*r?['"]([^'"\n]+)['"]"#).unwrap();

    let mut out: Vec<String> = Vec::new();
    let mut consumed_ranges: Vec<(usize, usize)> = Vec::new();

    for cap in triple.captures_iter(source) {
        #[allow(clippy::unwrap_used)] // group 0 is always Some inside captures_iter
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
        #[allow(clippy::unwrap_used)] // group 0 is always Some inside captures_iter
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
    #[allow(clippy::unwrap_used)] // static literal patterns — compile-time valid
    let with_www = Regex::new(
        r"https\??(?:s\?)?://(?:\(\?:www\\\.\)\?)([a-zA-Z0-9-]+(?:\\?\.[a-zA-Z0-9-]+)+)",
    )
    .unwrap();
    #[allow(clippy::unwrap_used)] // static literal pattern — compile-time valid
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

/// `yt_dlp/__init__.py` — see `build_from_ytdlp_templates/yt_dlp_init.py`.
const YT_DLP_INIT_PY: &str = include_str!("build_from_ytdlp_templates/yt_dlp_init.py");

/// `yt_dlp/extractor/common.py` — see `build_from_ytdlp_templates/extractor_common.py`.
const EXTRACTOR_COMMON_PY: &str = include_str!("build_from_ytdlp_templates/extractor_common.py");

/// `yt_dlp/utils/__init__.py` — see `build_from_ytdlp_templates/utils_init.py`.
const UTILS_INIT_PY: &str = include_str!("build_from_ytdlp_templates/utils_init.py");

/// `yt_dlp/utils/_utils.py` — re-exports the helpers that live in
/// `rdlp_ytdlp_compat._utils` and `rdlp_ytdlp_compat.info_extractor`.
/// See `build_from_ytdlp_templates/utils_helpers.py` for the full module mapping.
const UTILS_HELPERS_PY: &str = include_str!("build_from_ytdlp_templates/utils_helpers.py");

/// `yt_dlp/utils/traversal.py` — re-exports traversal helpers.
/// See `build_from_ytdlp_templates/utils_traversal.py` for the full module mapping.
const UTILS_TRAVERSAL_PY: &str = include_str!("build_from_ytdlp_templates/utils_traversal.py");

/// `_entry.py` (auto-generated wrapper) — see `build_from_ytdlp_templates/entry.py`.
/// `{{PLUGIN_MODULE}}` is substituted at stage time with the plugin's Python module name.
///
/// Load-bearing invariants (failing any will break dispatch):
///
/// 1. The class implementing `metadata`/`extract`/`search` MUST be named
///    `ExtractorPlugin` because componentize-py-pin@0.17.2 looks up a concrete
///    class whose name matches `--world-module` in `PascalCase`. Renaming it
///    produces `Can't instantiate abstract class ExtractorPlugin` at load.
/// 2. All imports stay at module top level (componentize-py issue #23 —
///    lazy `__import__()` silently fails inside the bundled `CPython`).
/// 3. Errors raise via `Err(<variant>)`, NOT return, because the WIT
///    Protocol method signature is the `Ok` payload only — see
///    `extractor_plugin/types.py::Err` (a frozen-dataclass Exception).
/// 4. Multi-class plugin support (Slice 2): `_entry.py` walks every
///    concrete `InfoExtractor` subclass in `user_plugin` at extract time
///    and dispatches by `cls.suitable(url)`. SVT-style siblings
///    (`SVTPlayIE` / `SVTSeriesIE` / `SVTPageIE`) ship in one .py and the
///    `suitable()` overrides decide which class claims a given URL.
///    Discovery + dispatch live in `rdlp_ytdlp_compat._dispatch` so they
///    are unit-testable in plain `CPython`.
/// 5. `info_dict` shape is validated per yt-dlp's documented contract
///    (`yt_dlp/extractor/common.py:107-498` at upstream tag 2026.03.17):
///    `id` and `title` are required strs; either `formats` or `url` must
///    be present.
/// 6. Python exceptions are mapped to WIT variants via a pure `isinstance`
///    ladder — see `_extractor_error_to_variant`. componentize-py 0.17.2
///    only marshals `Err.value` across the WIT boundary (`__cause__` is
///    dropped), so the dispatcher flattens one level of `__cause__` /
///    `cause` (yt-dlp legacy attr) plus `video_id` and `ie` into the
///    payload string at boundary crossing.
const ENTRY_TEMPLATE: &str = include_str!("build_from_ytdlp_templates/entry.py");
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
         wit_version = \"0.3.0\"\n\
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
        let first = patterns.first().expect("asserted non-empty");
        rdlp_plugin::dispatch::MatchPattern::parse(first).unwrap();
    }

    #[test]
    fn valid_url_unparseable_falls_back_to_wildcard() {
        let patterns =
            valid_urls_to_match_patterns(&[r"some-weird-regex-without-host".to_string()]);
        assert_eq!(patterns, vec!["*://*/*".to_string()]);
        let first = patterns.first().expect("asserted non-empty");
        rdlp_plugin::dispatch::MatchPattern::parse(first).unwrap();
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
        let src = r"
class SVTPlayIE(SVTBaseIE):
    _VALID_URL = r'''(?x)
                    (?:
                        svt:|
                        https?://(?:www\.)?svt\.se/foo/
                    )
                    (?P<id>[^/?#&]+)
                    '''
";
        let urls = extract_valid_urls(src);
        assert_eq!(urls.len(), 1);
        // Safety: asserted len == 1 above
        let first = urls.first().expect("asserted non-empty");
        assert!(first.contains("svt\\.se"), "got: {first:?}");
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
        let first = urls.first().expect("asserted non-empty");
        assert!(first.contains("real-foo"), "got {urls:?}");
    }

    #[test]
    fn extract_valid_urls_skips_single_quote_docstring_example() {
        // Same scenario, single-quoted docstring.
        let src = r"
class FooIE:
    '''Single-quoted docstring with example:
        _VALID_URL = r'https?://docstring\.example/(?P<id>\w+)'
    '''
    _VALID_URL = r'https?://real\.example/(?P<id>\w+)'
";
        let urls = extract_valid_urls(src);
        assert_eq!(urls.len(), 1, "got {urls:?}");
        let first = urls.first().expect("asserted non-empty");
        assert!(first.contains(r"real\.example"), "got {urls:?}");
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
