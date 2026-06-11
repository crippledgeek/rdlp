//! Temp-file naming for the download → finalize lifecycle (#406).
//!
//! The clean, user-visible output name never appears on disk until the work is
//! committed. The download writes a DETERMINISTIC `*.rdlp-part.*` name (so an
//! interrupted download is resumable across process restarts); the post-process
//! pipeline uses RANDOM `*.rdlp-tmp-{uuid}.*` names (defined in rdlp-postprocess).
//! A single finalize step renames the survivor back to the clean name.

use std::path::{Path, PathBuf};

/// Deterministic download-in-progress marker. Resumable; same-directory; never
/// carries a UUID (a random name would break resume on relaunch — see spec).
/// Its leading `.` is mirrored as `_` by `sanitize_filename` Step 1.5 to
/// neutralize title collisions — keep the two in sync.
pub(super) const PART_MARKER: &str = ".rdlp-part";

/// Pipeline-intermediate marker (random per stage; owned by rdlp-postprocess).
/// Mirrored here only so the finalize helper can strip either marker.
/// Its leading `.` is mirrored as `_` by `sanitize_filename` Step 1.5 to
/// neutralize title collisions — keep the two in sync.
pub(super) const TMP_MARKER: &str = ".rdlp-tmp-";

/// The stdout sentinel. Temp-naming is meaningless when output goes to stdout.
/// Matches ONLY the exact `-` token, not a path whose last component is `-`.
fn is_stdout(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Derive the deterministic `.rdlp-part` download path from the clean target.
///
/// `~/V/Title.mp4` → `~/V/Title.rdlp-part.mp4`. Always the SAME directory as the
/// clean target, so the finalize rename never crosses a filesystem boundary
/// (no `EXDEV`). Returns the input unchanged for the `-` stdout sentinel.
pub(super) fn part_path(clean: &Path) -> PathBuf {
    if is_stdout(clean) {
        return clean.to_path_buf();
    }
    let stem = clean
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let filename = match clean.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{stem}{PART_MARKER}.{ext}"),
        _ => format!("{stem}{PART_MARKER}"),
    };
    clean.with_file_name(filename)
}

/// Recover the clean stem from a temp-named file, stripping EITHER marker.
///
/// Uses marker-SEARCH (not a first-dot split) so dotted stems survive:
/// `My.Video.rdlp-tmp-abc.mkv` → `My.Video`. The `.rdlp-part` marker is
/// preferred; the pipeline `.rdlp-tmp-` marker is the fallback. Returns `None`
/// if neither marker is present (the name is already clean) or if a marker sits
/// at index 0 (no stem precedes it).
pub(super) fn strip_temp_marker(file_name: &str) -> Option<&str> {
    let idx = file_name
        .find(PART_MARKER)
        .or_else(|| file_name.find(TMP_MARKER))?;
    if idx == 0 {
        return None; // no real stem precedes the marker
    }
    Some(&file_name[..idx])
}

/// Derive the clean output path for a temp-named survivor: marker-stripped stem
/// plus the SURVIVOR's own extension (so a container change during post-processing,
/// e.g., ts → mkv, is preserved in the final name). Returns `None` for stdout or
/// for an already-clean name (no marker present).
#[allow(dead_code)] // wired into the finalize path in a later task
pub(super) fn finalize_to_clean(survivor: &Path) -> Option<PathBuf> {
    if is_stdout(survivor) {
        return None;
    }
    let name = survivor.file_name()?.to_str()?;
    let stem = strip_temp_marker(name)?;
    let clean = match survivor.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{stem}.{ext}"),
        _ => stem.to_owned(),
    };
    Some(survivor.with_file_name(clean))
}

/// Atomically commit a finished `.rdlp-part` download to its clean output name.
///
/// Same-directory rename (no `EXDEV`); on failure the `.rdlp-part` file is left
/// on disk (resumable) rather than a half-renamed state. Used by both the
/// already-complete resume short-circuit and the normal download-completion
/// path so the two share one error message and one rename call.
pub(super) async fn finalize_part(part: &Path, clean: &Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    tokio::fs::rename(part, clean).await.with_context(|| {
        format!(
            "failed to finalize download {} -> {}",
            part.display(),
            clean.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn part_path_inserts_marker_before_extension() {
        let clean = PathBuf::from("/home/u/Videos/Title.mp4");
        assert_eq!(
            part_path(&clean),
            PathBuf::from("/home/u/Videos/Title.rdlp-part.mp4")
        );
    }

    #[test]
    fn part_path_preserves_dotted_stem() {
        let clean = PathBuf::from("/v/My.Video.S01E01.mkv");
        assert_eq!(
            part_path(&clean),
            PathBuf::from("/v/My.Video.S01E01.rdlp-part.mkv")
        );
    }

    #[test]
    fn part_path_handles_no_extension() {
        let clean = PathBuf::from("/v/noext");
        assert_eq!(part_path(&clean), PathBuf::from("/v/noext.rdlp-part"));
    }

    #[test]
    fn part_path_is_noop_for_stdout() {
        let clean = PathBuf::from("-");
        assert_eq!(part_path(&clean), PathBuf::from("-"));
    }

    #[test]
    fn part_path_collapses_trailing_dot_to_no_extension() {
        let clean = PathBuf::from("/v/Title.");
        assert_eq!(part_path(&clean), PathBuf::from("/v/Title.rdlp-part"));
    }

    #[test]
    fn strip_recovers_stem_from_part_marker() {
        assert_eq!(strip_temp_marker("Title.rdlp-part.mp4"), Some("Title"));
    }

    #[test]
    fn strip_recovers_stem_from_tmp_marker() {
        assert_eq!(
            strip_temp_marker("Title.rdlp-tmp-abc123.mkv"),
            Some("Title")
        );
    }

    #[test]
    fn strip_preserves_dotted_stem() {
        assert_eq!(
            strip_temp_marker("My.Video.S01E01.rdlp-tmp-deadbeef.mkv"),
            Some("My.Video.S01E01")
        );
    }

    #[test]
    fn strip_returns_none_when_no_marker() {
        assert_eq!(strip_temp_marker("Title.mp4"), None);
    }

    #[test]
    fn finalize_uses_survivor_extension_for_container_change() {
        let survivor = PathBuf::from("/v/Title.rdlp-tmp-abc.mkv");
        assert_eq!(
            finalize_to_clean(&survivor),
            Some(PathBuf::from("/v/Title.mkv"))
        );
    }

    #[test]
    fn finalize_strips_part_marker() {
        let survivor = PathBuf::from("/v/Title.rdlp-part.mp4");
        assert_eq!(
            finalize_to_clean(&survivor),
            Some(PathBuf::from("/v/Title.mp4"))
        );
    }

    #[test]
    fn finalize_is_none_for_stdout_or_clean_name() {
        assert_eq!(finalize_to_clean(&PathBuf::from("-")), None);
        assert_eq!(finalize_to_clean(&PathBuf::from("/v/Title.mp4")), None);
    }

    #[test]
    fn strip_returns_none_for_marker_at_index_zero() {
        assert_eq!(strip_temp_marker(".rdlp-part.mp4"), None);
        // and finalize must therefore decline rather than produce a stemless name
        assert_eq!(finalize_to_clean(&PathBuf::from("/v/.rdlp-part.mp4")), None);
    }

    #[test]
    fn strip_prefers_part_marker_when_both_present() {
        // part is found first and wins the or_else chain
        assert_eq!(
            strip_temp_marker("Title.rdlp-part.rdlp-tmp-abc.mkv"),
            Some("Title")
        );
    }

    #[test]
    fn strip_truncates_when_stem_legitimately_contains_marker() {
        // Documented trade-off: a title literally containing the marker substring
        // is truncated at the first marker. sanitize_filename neutralizes this
        // upstream; this test pins the helper's raw behavior.
        assert_eq!(
            strip_temp_marker("Guide.rdlp-part.intro.mp4"),
            Some("Guide")
        );
    }
}
