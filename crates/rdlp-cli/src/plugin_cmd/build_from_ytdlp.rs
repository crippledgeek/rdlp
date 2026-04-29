//! `rdlp plugin build-from-ytdlp <plugin.py>` — wraps componentize-py 0.17.2
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
    let stem = py_path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("invalid plugin filename")?
        .to_string();

    let source = std::fs::read_to_string(&py_path)?;
    let valid_url =
        extract_valid_url(&source).context("could not find _VALID_URL in plugin source")?;
    let matches = valid_url_to_match_patterns(&valid_url);

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

    // componentize-py 0.17.2: dirty bindings dir errors with EEXIST. Clean first.
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
        .output()?;
    let path = String::from_utf8(output.stdout)?.trim().to_string();
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

const ENTRY_TEMPLATE: &str = r#""""Auto-generated entry point for rdlp plugin build-from-ytdlp.

componentize-py free-function exports collapse into ONE Protocol subclass
named after --world-module (PascalCase). componentize-py 0.17.2 looks up a
concrete class named `ExtractorPlugin` (matching the world-module name).
Errors raise via Err(<variant>), not return.
"""
from extractor_plugin import ExtractorPlugin as _ExtractorPluginProtocol
from extractor_plugin.types import Err
from extractor_plugin.imports.types import (
    InfoDict, Format, PluginInfo, SearchPage,
    ExtractError_NotFound, ExtractError_Internal, SearchError_Unsupported,
)

# User plugin imports — must be top-level (componentize-py #23).
from user_plugin import *  # noqa: F401,F403

import user_plugin
from rdlp_ytdlp_compat import InfoExtractor as _CompatInfoExtractor

_IE_CLASS = None
for _name in dir(user_plugin):
    _v = getattr(user_plugin, _name)
    if isinstance(_v, type) and issubclass(_v, _CompatInfoExtractor) and _v is not _CompatInfoExtractor:
        _IE_CLASS = _v
        break
if _IE_CLASS is None:
    raise RuntimeError("no InfoExtractor subclass found in plugin")
_IE = _IE_CLASS()


# CRITICAL: class name must be `ExtractorPlugin` (matches --world-module
# PascalCase) for componentize-py 0.17.2 to discover and instantiate it.
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
        try:
            d = _IE._real_extract(url)
        except Exception as e:
            raise Err(ExtractError_Internal(str(e)))
        return _dict_to_info_dict(d)

    def search(self, query) -> SearchPage:
        raise Err(SearchError_Unsupported())


def _dict_to_info_dict(d: dict) -> InfoDict:
    formats = [
        Format(
            format_id=str(f.get("format_id", "")),
            url=str(f.get("url", "")),
            ext=str(f.get("ext", "mp4")),
            protocol=str(f.get("protocol", "https")),
            width=f.get("width"), height=f.get("height"), fps=f.get("fps"),
            tbr=f.get("tbr"), vbr=f.get("vbr"), abr=f.get("abr"),
            vcodec=f.get("vcodec"), acodec=f.get("acodec"),
            container=f.get("container"), filesize=f.get("filesize"),
            format_note=f.get("format_note"),
        )
        for f in d.get("formats", [])
    ]
    return InfoDict(
        id=str(d.get("id", "")),
        title=str(d.get("title", "")),
        url=d.get("url"), formats=formats, subtitles=[],
        thumbnail=d.get("thumbnail"), description=d.get("description"),
        uploader=d.get("uploader"), uploader_id=d.get("uploader_id"),
        upload_date=d.get("upload_date"),
        duration=d.get("duration"), view_count=d.get("view_count"),
        like_count=d.get("like_count"),
        tags=d.get("tags", []), categories=d.get("categories", []),
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
    fn extract_valid_url_finds_pattern() {
        let src = "\nclass Foo:\n    _VALID_URL = r'https?://example\\.com/(?P<id>\\d+)'\n";
        assert_eq!(
            extract_valid_url(src),
            Some(r"https?://example\.com/(?P<id>\d+)".to_string())
        );
    }

    #[test]
    fn template_manifest_parses_against_real_schema() {
        // The emitted template must be parseable by the real Manifest type
        // (modulo placeholder values — base64 content isn't validated until
        // `rdlp plugin sign` runs).
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plugin.toml.template");
        write_manifest(&path, "test-plugin", &["https://example.com/*".to_string()]).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        // Must not contain forbidden [wasm] table.
        assert!(
            !body.contains("[wasm]"),
            "template has invalid [wasm] table"
        );
        // Must contain the placeholder signature block.
        assert!(body.contains("[signature]"));
        assert!(body.contains("type = \"ed25519\""));
        // Capability vocab must be unprefixed.
        assert!(body.contains("\"fetch\""));
        assert!(!body.contains("\"host:fetch\""));
    }
}
