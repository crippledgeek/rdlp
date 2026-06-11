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
#[allow(dead_code)] // used by finalize_to_clean in the next task
pub(super) const PART_MARKER: &str = ".rdlp-part";

/// Pipeline-intermediate marker (random per stage; owned by rdlp-postprocess).
/// Mirrored here only so the finalize helper can strip either marker.
#[allow(dead_code)] // used by finalize_to_clean in the next task
pub(super) const TMP_MARKER: &str = ".rdlp-tmp-";

/// The stdout sentinel. Temp-naming is meaningless when output goes to stdout.
/// Matches ONLY the exact `-` token, not a path whose last component is `-`.
#[allow(dead_code)] // used by part_path and finalize_to_clean
fn is_stdout(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Derive the deterministic `.rdlp-part` download path from the clean target.
///
/// `~/V/Title.mp4` → `~/V/Title.rdlp-part.mp4`. Always the SAME directory as the
/// clean target, so the finalize rename never crosses a filesystem boundary
/// (no `EXDEV`). Returns the input unchanged for the `-` stdout sentinel.
#[allow(dead_code)] // will be used by orchestrator::download in the next task
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
}
