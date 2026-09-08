//! Container format resolution with provenance tracking.
//!
//! Centralises all container-format decisions behind [`ResolvedContainer`],
//! which records *how* the format was chosen so callers can make informed
//! decisions and every fallback is explicitly logged.

use log::{debug, warn};
use rdlp_types::{ContainerFormat, ContainerRequest, ContainerSource};
use std::path::{Path, PathBuf};

/// A container format with provenance tracking.
///
/// The download/merge container decision flows through
/// [`resolve()`](Self::resolve), which applies the precedence rules and logs
/// fallbacks.
///
/// NOT every container decision in the codebase: post-processing stages ask
/// [`rdlp_types::PostProcess::explicit_container`] instead, whose chain also
/// covers the recode targets this one does not (they are irrelevant here — a
/// recode happens after the download container is already chosen). The two
/// share [`ContainerSource`]/[`ContainerRequest`] so the provenance vocabulary
/// is one definition, but they answer different questions and their precedence
/// deliberately differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedContainer {
    /// The resolved container format.
    pub format: ContainerFormat,
    /// How this format was determined.
    pub source: ContainerSource,
}

impl ResolvedContainer {
    /// Resolve the target container using the precedence chain.
    ///
    /// # Precedence (highest to lowest)
    /// 1. `config.postprocess.remux_container` (`--remux=<fmt>`)
    /// 2. `config.postprocess.merge_output_format` (config TOML / API only)
    /// 3. Output file extension (from format selection)
    /// 4. Fallback to MP4 with warning
    pub fn resolve(config: &rdlp_types::Config, output_path: Option<&Path>) -> Self {
        // Priority 1: explicit remux target
        if let Some(c) = config.postprocess.remux_container {
            return Self {
                format: c,
                source: ContainerSource::Requested(ContainerRequest::Remux),
            };
        }

        // Priority 2: explicit merge output format
        if let Some(c) = config.postprocess.merge_output_format {
            return Self {
                format: c,
                source: ContainerSource::Requested(ContainerRequest::MergeOutputFormat),
            };
        }

        // Priority 3: infer from output file extension
        if let Some(path) = output_path
            && let Some(c) = ContainerFormat::from_path(path)
        {
            // Skip .ts — raw MPEG-TS is an intermediate format
            if c != ContainerFormat::Ts {
                return Self {
                    format: c,
                    source: ContainerSource::FileExtension,
                };
            }
        }

        // Priority 4: fallback (common for HLS where output starts as .ts)
        debug!("No container preference set; falling back to MP4");
        Self {
            format: ContainerFormat::Mp4,
            source: ContainerSource::Fallback,
        }
    }
}

/// Build a stub output path for subtitle-only downloads.
///
/// When downloading subtitles without a video file, we need a path to
/// derive subtitle filenames from (stem + lang + ext). Uses
/// [`ResolvedContainer`] to avoid hardcoding a container extension.
pub fn output_stub(
    config: &rdlp_types::Config,
    output_dir: &Path,
    sanitized_title: &str,
) -> PathBuf {
    let resolved = ResolvedContainer::resolve(config, None);
    output_dir.join(format!("{sanitized_title}.{}", resolved.format.as_ext()))
}

/// Placeholder for a sidecar filename segment that sanitization removed.
///
/// `und` is ISO 639-2/639-3 for "undetermined language" (the value
/// Matroska and MP4 use for an untagged track), which is what an empty or
/// unrepresentable subtitle language is. Reused for any lost segment
/// because the alternative — omitting it — makes `{lang}.{ext}` collapse
/// into a bare `{ext}`, so a subtitle track can be written over the media
/// file (`video.mkv`), the thumbnail that is later embedded (`video.jpg`),
/// or the resume state the next run parses (`video.rdlp_state.json`).
const UNKNOWN_SEGMENT: &str = "und";

/// Derive a sidecar file path from an output path.
///
/// Extracts the stem and parent directory from `base_path`, then appends
/// `suffix` to form a sidecar filename. Replaces manual stem+parent+join
/// patterns for subtitles, thumbnails, and other companion files.
///
/// `suffix` can be `"jpg"`, `"en.srt"`, `"rdlp_state.json"`, etc.
///
/// # Security
///
/// `suffix` is remote-controlled on the subtitle path — it is built from
/// `{track.language}.{track.ext}`, and `language` is the raw
/// `InfoDict.subtitles` map key, which some extractors take verbatim from
/// site JSON. It is sanitized here rather than at each call site, so the
/// suffix contributes exactly one path component: no separators, no `..`.
/// (A caller's own `base_path` is passed through as given — `sidecar_path`
/// constrains the suffix, not the directory it is handed.)
///
/// Sanitizing can also *shorten* a suffix, which is its own hazard: a
/// subtitle whose language sanitizes away would collapse `{lang}.{ext}` to
/// a bare `{ext}` and land on one of rdlp's own files. See
/// [`UNKNOWN_SEGMENT`].
pub fn sidecar_path(base_path: &Path, suffix: &str) -> PathBuf {
    let stem = base_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let parent = base_path.parent().unwrap_or_else(|| Path::new("."));
    let sanitized = super::Orchestrator::sanitize_filename(suffix);

    // `sanitize_filename` only ever removes characters, so a `.`-delimited
    // segment can disappear entirely — an empty, dot-only or space-only
    // subtitle language turns `.mkv` into `mkv`. That collapse is what lets
    // a subtitle impersonate a different sidecar species, so restore a
    // placeholder segment whenever the count drops.
    let restored = if sanitized.split('.').count() < suffix.split('.').count() {
        format!("{UNKNOWN_SEGMENT}.{sanitized}")
    } else {
        sanitized
    };

    if restored != suffix {
        // Otherwise the file silently appears under a name the operator
        // never asked for, and a colliding track looks like a resume hit.
        warn!(
            requested:% = suffix.escape_debug(),
            used:% = restored;
            "Sidecar suffix was sanitized"
        );
    }

    let path = parent.join(format!("{stem}.{restored}"));

    // Belt-and-braces: a sidecar must never *be* the file it accompanies.
    // The segment restore above already prevents the reachable route here
    // (`.mkv` → `und.mkv`); this catches any future caller that passes a
    // bare suffix equal to `base_path`'s own extension.
    if path == base_path {
        parent.join(format!("{stem}.{UNKNOWN_SEGMENT}.{restored}"))
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn base_config() -> rdlp_types::Config {
        rdlp_types::Config {
            postprocess: rdlp_types::PostProcess {
                remux_container: None,
                merge_output_format: None,
                ..rdlp_types::PostProcess::default()
            },
            ..rdlp_types::Config::default()
        }
    }

    #[test]
    fn test_resolve_remux_wins_over_all() {
        let mut config = base_config();
        config.postprocess.remux_container = Some(ContainerFormat::Mkv);
        config.postprocess.merge_output_format = Some(ContainerFormat::Mp4);
        let path = PathBuf::from("/tmp/video.webm");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(
            r.source,
            ContainerSource::Requested(ContainerRequest::Remux)
        );
    }

    #[test]
    fn test_resolve_merge_wins_over_extension() {
        let mut config = base_config();
        config.postprocess.merge_output_format = Some(ContainerFormat::Mkv);
        let path = PathBuf::from("/tmp/video.mp4");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(
            r.source,
            ContainerSource::Requested(ContainerRequest::MergeOutputFormat)
        );
    }

    #[test]
    fn test_resolve_extension_wins_over_fallback() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video.mkv");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(r.source, ContainerSource::FileExtension);
    }

    #[test]
    fn test_resolve_fallback_when_nothing_set() {
        let config = base_config();
        let r = ResolvedContainer::resolve(&config, None);
        assert_eq!(r.format, ContainerFormat::Mp4);
        assert_eq!(r.source, ContainerSource::Fallback);
    }

    #[test]
    fn test_resolve_ts_extension_skipped() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video.ts");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mp4);
        assert_eq!(r.source, ContainerSource::Fallback);
    }

    #[test]
    fn test_resolve_no_path_uses_config() {
        let mut config = base_config();
        config.postprocess.merge_output_format = Some(ContainerFormat::Mkv);
        let r = ResolvedContainer::resolve(&config, None);
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(
            r.source,
            ContainerSource::Requested(ContainerRequest::MergeOutputFormat)
        );
    }

    #[test]
    fn test_resolve_unknown_extension_fallback() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video.xyz");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mp4);
        assert_eq!(r.source, ContainerSource::Fallback);
    }

    #[test]
    fn test_resolve_empty_extension_fallback() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mp4);
        assert_eq!(r.source, ContainerSource::Fallback);
    }

    #[test]
    fn test_resolve_case_insensitive() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video.MKV");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(r.source, ContainerSource::FileExtension);
    }

    /// Regression test: HLS downloads previously hardcoded MP4 regardless
    /// of the format the user selected from the interactive menu.
    #[test]
    fn test_resolve_respects_mkv_extension_regression() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video.mkv");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(r.source, ContainerSource::FileExtension);
    }

    /// Verify webm files resolve correctly (not silently converted to mp4)
    #[test]
    fn test_resolve_respects_webm_extension() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video.webm");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::WebM);
        assert_eq!(r.source, ContainerSource::FileExtension);
    }

    /// Verify mov files resolve correctly
    #[test]
    fn test_resolve_respects_mov_extension() {
        let config = base_config();
        let path = PathBuf::from("/tmp/video.mov");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mov);
        assert_eq!(r.source, ContainerSource::FileExtension);
    }

    // ── sidecar_path tests ──────────────────────────────────

    #[test]
    fn test_sidecar_path_subtitle() {
        let base = PathBuf::from("/tmp/video.mkv");
        assert_eq!(
            sidecar_path(&base, "en.srt"),
            PathBuf::from("/tmp/video.en.srt")
        );
    }

    #[test]
    fn test_sidecar_path_thumbnail() {
        let base = PathBuf::from("/tmp/video.mp4");
        assert_eq!(sidecar_path(&base, "jpg"), PathBuf::from("/tmp/video.jpg"));
    }

    #[test]
    fn test_sidecar_path_no_extension() {
        let base = PathBuf::from("/tmp/video");
        assert_eq!(
            sidecar_path(&base, "en.srt"),
            PathBuf::from("/tmp/video.en.srt")
        );
    }

    #[test]
    fn test_sidecar_path_no_parent() {
        let base = PathBuf::from("video.mp4");
        assert_eq!(sidecar_path(&base, "jpg"), PathBuf::from("video.jpg"));
    }

    // ── sidecar_path suffix-injection tests ─────────────────
    //
    // `suffix` is remote-controlled: for subtitles it is
    // `{track.language}.{track.ext}`, and `language` is the raw
    // `InfoDict.subtitles` map key (for 9anime, the site's JSON
    // `track.label` verbatim). A separator or `..` in it would otherwise
    // reach the `tokio::fs::write` target in `subtitle/download.rs`.

    /// The sidecar must be a single file directly inside the base path's
    /// parent, still named after the base path's stem — asserted on the
    /// resolved path, not on a substring.
    ///
    /// The stem check is the load-bearing half: a `file_name().is_some()`
    /// assertion cannot fail (`file_name` never yields an empty `OsStr`),
    /// whereas requiring the `{stem}.` prefix would catch a future change
    /// that let the suffix eat the stem.
    fn assert_inside_parent(path: &Path, parent: &Path, stem: &str) {
        assert_eq!(path.parent(), Some(parent), "escaped parent dir: {path:?}");
        assert!(
            !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
            "retains a `..` component: {path:?}"
        );
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("sidecar file name is valid UTF-8");
        assert!(
            name.starts_with(&format!("{stem}.")),
            "suffix displaced the stem `{stem}`: {name:?}"
        );
    }

    #[test]
    fn sidecar_path_neutralizes_traversal_suffix() {
        let base = PathBuf::from("/tmp/out/video.mkv");
        // language = "en/../../pwn", ext = "vtt"
        let path = sidecar_path(&base, "en/../../pwn.vtt");
        assert_inside_parent(&path, Path::new("/tmp/out"), "video");
    }

    /// A leading `/` never produced an *absolute* path: the stem is always
    /// formatted in front of the suffix, so the joined string stays
    /// relative regardless. What it did produce was injected directories.
    #[test]
    fn sidecar_path_neutralizes_leading_separator_suffix() {
        let base = PathBuf::from("/tmp/out/video.mkv");
        let path = sidecar_path(&base, "/etc/cron.d/pwn.vtt");
        assert_inside_parent(&path, Path::new("/tmp/out"), "video");
    }

    #[test]
    fn sidecar_path_neutralizes_windows_separator_suffix() {
        let base = PathBuf::from("/tmp/out/video.mkv");
        let path = sidecar_path(&base, r"en\..\..\pwn.vtt");
        assert_inside_parent(&path, Path::new("/tmp/out"), "video");
        let name = path.file_name().and_then(|n| n.to_str()).unwrap();
        assert!(!name.contains('\\'), "kept a backslash: {name}");
    }

    #[test]
    fn sidecar_path_strips_nul_and_control_characters() {
        let base = PathBuf::from("/tmp/out/video.mkv");
        let path = sidecar_path(&base, "en\u{0}\u{7}\u{1b}[31m.srt");
        assert_inside_parent(&path, Path::new("/tmp/out"), "video");
        let name = path.file_name().and_then(|n| n.to_str()).unwrap();
        assert!(
            !name.chars().any(|c| c == '\0' || c.is_control()),
            "kept a control character: {name:?}"
        );
    }

    // ── sidecar_path segment-collapse tests ─────────────────
    //
    // Sanitizing shortens as well as replaces. A language that sanitizes
    // away collapses `{lang}.{ext}` into a bare `{ext}`, which would let a
    // subtitle land on one of rdlp's own files.

    #[test]
    fn sidecar_path_empty_language_does_not_hit_the_media_file() {
        let base = PathBuf::from("/tmp/out/video.mkv");
        // language = "", ext = "mkv" — up to 5 MB of subtitle bytes would
        // otherwise be written straight to the final output path.
        let path = sidecar_path(&base, ".mkv");
        assert_ne!(path, base, "subtitle overwrote the media file");
        assert_eq!(path, PathBuf::from("/tmp/out/video.und.mkv"));
    }

    #[test]
    fn sidecar_path_blank_language_does_not_hit_rdlp_sidecars() {
        let base = PathBuf::from("/tmp/out/video.mkv");
        for (suffix, label) in [
            (".jpg", "thumbnail"),
            (".rdlp_state.json", "resume state"),
            ("..srt", "dot-only language"),
            ("  .srt", "space-only language"),
        ] {
            let path = sidecar_path(&base, suffix);
            assert_inside_parent(&path, Path::new("/tmp/out"), "video");
            let name = path.file_name().and_then(|n| n.to_str()).unwrap();
            assert!(
                name.starts_with("video.und."),
                "{label}: lost the language segment: {name:?}"
            );
        }
    }

    #[test]
    fn sidecar_path_never_equals_base_path() {
        // A caller passing a bare suffix equal to the base extension is the
        // route the segment restore does not cover.
        let base = PathBuf::from("/tmp/out/video.mkv");
        assert_ne!(sidecar_path(&base, "mkv"), base);
    }

    #[test]
    fn sidecar_path_distinguishes_colliding_languages_from_the_media_file() {
        // Sanitization is many-to-one, so these two DO collide with each
        // other — that collision is handled at the subtitle call sites.
        // What must not happen is either of them hitting the media file.
        let base = PathBuf::from("/tmp/out/video.mkv");
        let a = sidecar_path(&base, "en/x.srt");
        let b = sidecar_path(&base, "en:x.srt");
        assert_eq!(a, b, "expected the documented many-to-one collision");
        assert_ne!(a, base);
    }

    #[test]
    fn sidecar_path_preserves_legitimate_language_tags() {
        let base = PathBuf::from("/tmp/out/video.mkv");
        for (suffix, expected) in [
            ("en.srt", "/tmp/out/video.en.srt"),
            ("pt-BR.srt", "/tmp/out/video.pt-BR.srt"),
            ("zh-Hans.vtt", "/tmp/out/video.zh-Hans.vtt"),
            ("jpg", "/tmp/out/video.jpg"),
            ("rdlp_state.json", "/tmp/out/video.rdlp_state.json"),
        ] {
            assert_eq!(sidecar_path(&base, suffix), PathBuf::from(expected));
        }
    }

    // ── output_stub tests ───────────────────────────────────

    #[test]
    fn test_output_stub_uses_resolver() {
        let mut config = base_config();
        config.postprocess.remux_container = Some(ContainerFormat::Mkv);
        let stub = output_stub(&config, std::path::Path::new("/tmp"), "My Video");
        assert_eq!(stub, PathBuf::from("/tmp/My Video.mkv"));
    }

    #[test]
    fn test_output_stub_default_mp4() {
        let config = base_config();
        let stub = output_stub(&config, std::path::Path::new("/tmp"), "My Video");
        assert_eq!(stub, PathBuf::from("/tmp/My Video.mp4"));
    }
}
