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
/// `suffix` can be `"jpg"`, `"en.srt"`, `"video.f137.mp4"`, etc. It may NOT
/// be a name rdlp reserves for itself — a suffix composing the session-state
/// file or a temp marker is defused rather than honoured (see below), so this
/// is not the route to build one of those.
///
/// # Security
///
/// `suffix` is remote-controlled on the subtitle path — it is built from
/// `{track.language}.{track.ext}`, and `language` is the raw
/// `InfoDict.subtitles` map key, which some extractors take verbatim from
/// site JSON. It is sanitized here rather than at each call site, so the
/// suffix contributes exactly one path component: no separators, no `..`.
/// The *directory* of `base_path` is passed through as given — this
/// constrains the file name, not where the caller puts it.
///
/// Sanitizing can also *shorten* a suffix, which is its own hazard: a
/// subtitle whose language sanitizes away would collapse `{lang}.{ext}` to
/// a bare `{ext}` and land on one of rdlp's own files. See
/// [`UNKNOWN_SEGMENT`].
///
/// # The stem is not passed through verbatim
///
/// The reserved-name rewrite runs over the *joined* name — it has to, since
/// that is where the joining dot can create a marker neither part contained
/// — so a `base_path` whose own stem spells a marker has that stem rewritten
/// too (`Title.rdlp-tmp-abc.mkv` + `jpg` → `Title_rdlp-tmp-abc.jpg`), and
/// the sidecar then no longer shares its media file's stem.
///
/// This is a deliberate trade, not an oversight. The guarantee that no
/// output of `sidecar_path` ever spells a reserved name is worth more than
/// an untouched-stem promise: `suffix` is remote-controlled while
/// `base_path` is rdlp-owned, and all four call sites pass template-generated
/// paths that already went through `sanitize_filename`, so the rewrite is
/// unreachable for the stem today. A caller that hands in a genuinely
/// marker-named `base_path` gets a sidecar that no longer pairs with it
/// under the `{stem}.` discovery in `find_subtitle_files` / `original_stem_for`.
pub fn sidecar_path(base_path: &Path, suffix: &str) -> PathBuf {
    let stem = base_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let parent = base_path.parent().unwrap_or_else(|| Path::new("."));
    let sanitized = super::Orchestrator::sanitize_filename(suffix);

    // `sanitize_filename` never *inserts* a `.`, and has two ways of taking
    // one away: it trims leading/trailing dots, and it rewrites a marker's
    // dot to an underscore. So a drop in the `.`-delimited segment count
    // means a segment vanished — an empty, dot-only or space-only subtitle
    // language turns `.mkv` into `mkv`. That collapse is what lets a subtitle
    // impersonate a different sidecar species, so restore a placeholder.
    let restored = if sanitized.split('.').count() < suffix.split('.').count() {
        format!("{UNKNOWN_SEGMENT}.{sanitized}")
    } else {
        sanitized
    };

    // Every reserved name is dot-PREFIXED, so a placeholder segment in front
    // never breaks one — `und.rdlp-part.mp4` still spells a marker, and
    // `{stem}.und.rdlp_state.json` is still a state file, just of the
    // download named `{stem}.und`. The remedy is the same dot→underscore
    // rewrite `sanitize_filename` already applies, run over the JOINED name:
    // a suffix that merely *starts* with a reserved token carries none when
    // the sanitizer inspects the suffix alone, and the dot joined in here
    // reconstitutes one. Left unguarded, `language="rdlp-part", ext="mp4"`
    // composes exactly `naming::part_path`, which resume probes and then
    // trusts the bytes of.
    let compose = |sfx: &str| {
        let joined = super::Orchestrator::neutralize_temp_markers(&format!("{stem}.{sfx}"));
        // The state file is not a temp marker, so it is not the sanitizer's
        // business — but it is the same shape, and `single_video_state_path`
        // builds it with its own join and never routes through here.
        let state = super::session_state::STATE_SUFFIX;
        joined.replace(state, &state.replacen('.', "_", 1))
    };

    // Belt-and-braces: a sidecar must never *be* the file it accompanies.
    // `thumbnail.rs` passes a bare, dotless suffix today (`sidecar_path(media_file,
    // &ext)`), so a thumbnail whose detected extension matches the media
    // container's is exactly this case — not a hypothetical future caller.
    let mut file_name = compose(&restored);
    if parent.join(&file_name) == base_path {
        file_name = compose(&format!("{UNKNOWN_SEGMENT}.{restored}"));
    }

    if file_name != format!("{stem}.{suffix}") {
        // Otherwise the file silently appears under a name the operator
        // never asked for, and a colliding track looks like a resume hit.
        warn!(
            requested:% = format!("{stem}.{suffix}").escape_debug(),
            used:% = file_name.escape_debug();
            "Sidecar name was sanitized"
        );
    }

    parent.join(file_name)
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
            // `video.und` rather than `video.und.`: for the state-file case
            // the reserved-name rewrite additionally turns the separator that
            // follows into `_` (`video.und_rdlp_state.json`). What matters
            // here is that the lost segment came back, not what delimits it.
            assert!(
                name.starts_with("video.und"),
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
            // `rdlp_state.json` used to be listed here as a legitimate
            // pass-through. It is not: it composes the session-state file's
            // exact name, so this test was pinning the defect the reserved-name
            // guard below now closes.
            ("srt", "/tmp/out/video.srt"),
        ] {
            assert_eq!(sidecar_path(&base, suffix), PathBuf::from(expected));
        }
    }

    // ── sidecar_path reserved-name tests ────────────────────
    //
    // `sanitize_filename` neutralizes a temp marker only where it can SEE
    // one, and the markers are dot-prefixed. A suffix that merely *starts*
    // with `rdlp-part` carries no marker until `sidecar_path` joins the stem
    // on with a dot — so the sanitizer is a no-op, the segment count never
    // drops, and the composed name is a file rdlp owns.

    #[test]
    fn sidecar_path_cannot_forge_a_part_file() {
        let base = PathBuf::from("/tmp/out/Title.mp4");
        // language = "rdlp-part", ext = "mp4"
        let path = sidecar_path(&base, "rdlp-part.mp4");
        assert_ne!(
            path,
            crate::orchestrator::naming::part_path(&base),
            "composed the resume-probed in-progress name"
        );
        let name = path.file_name().and_then(|n| n.to_str()).unwrap();
        assert!(
            !name.contains(crate::orchestrator::naming::PART_MARKER),
            "name still spells the part marker: {name:?}"
        );
    }

    #[test]
    fn sidecar_path_cannot_forge_pipeline_or_backup_names() {
        let base = PathBuf::from("/tmp/out/Title.mp4");
        for (suffix, marker) in [
            (
                "rdlp-tmp-abc123.mp4",
                crate::orchestrator::naming::TMP_MARKER,
            ),
            (
                "rdlp-bak-abc123.mp4",
                crate::orchestrator::naming::BAK_MARKER,
            ),
        ] {
            let path = sidecar_path(&base, suffix);
            let name = path.file_name().and_then(|n| n.to_str()).unwrap();
            assert!(
                !name.contains(marker),
                "name still spells {marker}: {name:?}"
            );
        }
    }

    #[test]
    fn sidecar_path_cannot_forge_the_session_state_file() {
        let base = PathBuf::from("/tmp/out/Title.mp4");
        // language = "rdlp_state", ext = "json" — byte-identical through the
        // sanitizer, so only the reserved-name rewrite catches it.
        let path = sidecar_path(&base, "rdlp_state.json");
        assert_ne!(
            path,
            crate::orchestrator::session_state::single_video_state_path(
                Path::new("/tmp/out"),
                "Title"
            ),
            "composed the session-state file's own name"
        );
    }

    /// A reserved name is an *ending*, not just a stem-exact spelling: a
    /// state file belongs to whichever download its leading text names, so
    /// `{stem}.en.rdlp_state.json` is the state of `{stem}.en`. A guard
    /// keyed on this download's own stem — or a placeholder segment in
    /// front, which only renames the victim — would miss it.
    #[test]
    fn sidecar_path_cannot_forge_another_downloads_state_file() {
        let base = PathBuf::from("/tmp/out/Title.mp4");
        // language = "en", ext = "rdlp_state.json"
        let path = sidecar_path(&base, "en.rdlp_state.json");
        assert_ne!(
            path,
            crate::orchestrator::session_state::single_video_state_path(
                Path::new("/tmp/out"),
                "Title.en"
            ),
            "composed another download's session-state name"
        );
        let name = path.file_name().and_then(|n| n.to_str()).unwrap();
        assert!(
            !name.ends_with(crate::orchestrator::session_state::STATE_SUFFIX),
            "name still ends with the state-file spelling: {name:?}"
        );
    }

    /// The marker rewrite must not fire on an ordinary name that merely
    /// mentions rdlp.
    ///
    /// `rdlp_part` (underscore) is deliberately not `rdlp-partial`: the
    /// marker is matched as a substring, so `.rdlp-partial` *does* contain
    /// `.rdlp-part` and is rewritten.
    ///
    /// That over-match is required, not tolerated. `naming::strip_temp_marker`
    /// locates markers with a plain `find`, so it reads `demo.rdlp-partial.mp4`
    /// as a part file — the neutralizer must over-match identically, or a name
    /// the stripper still treats as a marker would slip through undefused.
    /// Narrowing either side to a word boundary recreates this commit's bug
    /// class.
    #[test]
    fn sidecar_path_leaves_marker_lookalikes_alone() {
        let base = PathBuf::from("/tmp/out/Title.mp4");
        assert_eq!(
            sidecar_path(&base, "rdlp_part.srt"),
            PathBuf::from("/tmp/out/Title.rdlp_part.srt")
        );
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
