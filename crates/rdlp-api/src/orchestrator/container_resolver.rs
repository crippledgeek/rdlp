//! Container format resolution with provenance tracking.
//!
//! Centralises all container-format decisions behind [`ResolvedContainer`],
//! which records *how* the format was chosen so callers can make informed
//! decisions and every fallback is explicitly logged.

use log::warn;
use rdlp_core::ContainerFormat;
use std::path::{Path, PathBuf};

/// How the container format was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerSource {
    /// User explicitly set via `--remux=<container>`.
    RemuxConfig,
    /// User explicitly set via `--merge-output-format=<container>`.
    MergeConfig,
    /// Inferred from the output file's extension (reflects format selection).
    FileExtension,
    /// No user preference; fell back to a safe default.
    Fallback,
}

/// A container format with provenance tracking.
///
/// Every container decision flows through [`resolve()`](Self::resolve),
/// which applies the precedence rules and logs fallbacks.
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
    /// 1. `config.remux_container` (`--remux=<fmt>`)
    /// 2. `config.merge_output_format` (`--merge-output-format=<fmt>`)
    /// 3. Output file extension (from format selection)
    /// 4. Fallback to MP4 with warning
    pub fn resolve(config: &rdlp_core::Config, output_path: Option<&Path>) -> Self {
        // Priority 1: explicit remux target
        if let Some(c) = config.remux_container {
            return Self {
                format: c,
                source: ContainerSource::RemuxConfig,
            };
        }

        // Priority 2: explicit merge output format
        if let Some(c) = config.merge_output_format {
            return Self {
                format: c,
                source: ContainerSource::MergeConfig,
            };
        }

        // Priority 3: infer from output file extension
        if let Some(path) = output_path {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if let Ok(c) = ext.parse::<ContainerFormat>() {
                    // Skip .ts — raw MPEG-TS is an intermediate format
                    if c != ContainerFormat::Ts {
                        return Self {
                            format: c,
                            source: ContainerSource::FileExtension,
                        };
                    }
                }
            }
        }

        // Priority 4: fallback
        warn!("No container preference set; falling back to MP4");
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
    config: &rdlp_core::Config,
    output_dir: &Path,
    sanitized_title: &str,
) -> PathBuf {
    let resolved = ResolvedContainer::resolve(config, None);
    output_dir.join(format!("{sanitized_title}.{}", resolved.format.as_ext()))
}

/// Derive a sidecar file path from an output path.
///
/// Extracts the stem and parent directory from `base_path`, then appends
/// `suffix` to form a sidecar filename. Replaces manual stem+parent+join
/// patterns for subtitles, thumbnails, and other companion files.
///
/// `suffix` can be `"jpg"`, `"en.srt"`, `"rdlp_state.json"`, etc.
pub fn sidecar_path(base_path: &Path, suffix: &str) -> PathBuf {
    let stem = base_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let parent = base_path.parent().unwrap_or(Path::new("."));
    parent.join(format!("{stem}.{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn base_config() -> rdlp_core::Config {
        rdlp_core::Config {
            remux_container: None,
            merge_output_format: None,
            ..rdlp_core::Config::default()
        }
    }

    #[test]
    fn test_resolve_remux_wins_over_all() {
        let mut config = base_config();
        config.remux_container = Some(ContainerFormat::Mkv);
        config.merge_output_format = Some(ContainerFormat::Mp4);
        let path = PathBuf::from("/tmp/video.webm");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(r.source, ContainerSource::RemuxConfig);
    }

    #[test]
    fn test_resolve_merge_wins_over_extension() {
        let mut config = base_config();
        config.merge_output_format = Some(ContainerFormat::Mkv);
        let path = PathBuf::from("/tmp/video.mp4");
        let r = ResolvedContainer::resolve(&config, Some(&path));
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(r.source, ContainerSource::MergeConfig);
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
        config.merge_output_format = Some(ContainerFormat::Mkv);
        let r = ResolvedContainer::resolve(&config, None);
        assert_eq!(r.format, ContainerFormat::Mkv);
        assert_eq!(r.source, ContainerSource::MergeConfig);
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

    // ── output_stub tests ───────────────────────────────────

    #[test]
    fn test_output_stub_uses_resolver() {
        let mut config = base_config();
        config.remux_container = Some(ContainerFormat::Mkv);
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
