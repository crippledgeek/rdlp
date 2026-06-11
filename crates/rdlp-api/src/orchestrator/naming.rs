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
    // SAFETY: PART_MARKER and TMP_MARKER are ASCII, so `find` returns an index
    // on a UTF-8 char boundary; the byte-slice below can never split a codepoint.
    debug_assert!(file_name.is_char_boundary(idx));
    Some(&file_name[..idx])
}

/// Derive the clean output path for a temp-named survivor: marker-stripped stem
/// plus the SURVIVOR's own extension (so a container change during post-processing,
/// e.g., ts → mkv, is preserved in the final name). Returns `None` for stdout or
/// for an already-clean name (no marker present).
pub fn finalize_to_clean(survivor: &Path) -> Option<PathBuf> {
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

/// Atomically commit a finished temp-named file to its clean output name.
///
/// Same-directory rename (no `EXDEV`). On POSIX `rename(2)` replaces an
/// existing destination atomically. On Windows `MoveFileExW` fails if the
/// destination exists unless `MOVEFILE_REPLACE_EXISTING` is set; `tokio::fs::rename`
/// does NOT set that flag, so we must handle a pre-existing clean file before
/// renaming. This matters for the borrowed-input (`keep_inputs=true`) same-
/// container in-place case: the source is still on disk when finalize runs and
/// the survivor must replace it (#414).
///
/// Uses a **backup-restore** pattern when `clean` already exists: the existing
/// file is moved aside first, and restored if the final rename fails, so `clean`
/// is never lost without `part` safely in place.
///
/// On failure the temp file is left intact and the error is surfaced to the
/// caller via context.
pub async fn finalize_part(part: &Path, clean: &Path) -> anyhow::Result<()> {
    use anyhow::Context as _;

    if part != clean && tokio::fs::try_exists(clean).await.unwrap_or(false) {
        // `clean` already exists (e.g. keep_inputs same-container in-place case).
        // A bare remove-then-rename has a data-loss window: if the rename fails
        // after the remove, `clean` is gone forever. Move the existing file aside
        // first and restore it if the rename fails, so `clean` is never lost.
        let backup = {
            let mut name = clean
                .file_name()
                .map(std::ffi::OsStr::to_os_string)
                .unwrap_or_default();
            name.push(format!(".rdlp-bak-{}", uuid::Uuid::new_v4().simple()));
            clean.with_file_name(name)
        };
        tokio::fs::rename(clean, &backup)
            .await
            .with_context(|| format!("failed to back up existing {}", clean.display()))?;
        match tokio::fs::rename(part, clean).await {
            Ok(()) => {
                // Success: drop the backup (best-effort; ignore errors).
                let _ = tokio::fs::remove_file(&backup).await;
                Ok(())
            }
            Err(e) => {
                // Restore the original so `clean` is never lost.
                let _ = tokio::fs::rename(&backup, clean).await;
                Err(e).with_context(|| {
                    format!(
                        "failed to finalize {} -> {} (original restored)",
                        part.display(),
                        clean.display()
                    )
                })
            }
        }
    } else {
        tokio::fs::rename(part, clean).await.with_context(|| {
            format!(
                "failed to finalize download {} -> {}",
                part.display(),
                clean.display()
            )
        })
    }
}

/// Best-effort removal of a `.rdlp-part` working file on explicit cancel, so a
/// cancelled download leaves no artifact (#406). Errors are ignored: the file
/// may legitimately not exist yet, and a failed unlink must not mask the cancel.
pub(super) async fn discard_part(part: &Path) {
    let _ = tokio::fs::remove_file(part).await;
}

/// Derive a pipeline-namespace temp path `{clean_stem}.rdlp-tmp-{uuid}.{ext}`
/// from the clean target, in the SAME directory. Used at the download->pipeline
/// seam so the post-process `FileTracker` only ever sees its own `.rdlp-tmp-`
/// namespace (its marker logic and hardened cancel/Drop path stay untouched).
/// The clean stem is recovered via marker-search, so a `.rdlp-part` input or a
/// clean target both yield the same stem. Returns the input unchanged for the
/// `-` stdout sentinel.
pub fn seam_path(clean: &Path) -> PathBuf {
    if is_stdout(clean) {
        return clean.to_path_buf();
    }
    let raw = clean
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    // If `clean` was actually a `.rdlp-part` name, recover the true stem.
    let stem = strip_temp_marker(raw).unwrap_or(raw);
    let id = uuid::Uuid::new_v4().simple().to_string();
    let filename = match clean.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{stem}{TMP_MARKER}{id}.{ext}"),
        _ => format!("{stem}{TMP_MARKER}{id}"),
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

    #[test]
    fn seam_path_is_in_tmp_namespace_same_dir_and_ext() {
        let clean = PathBuf::from("/v/My.Video.mp4");
        let seam = seam_path(&clean);
        let name = seam.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("My.Video.rdlp-tmp-"), "got: {name}");
        assert!(
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4")),
            "got: {name}"
        );
        assert_eq!(seam.parent(), clean.parent());
        // the segment between ".rdlp-tmp-" and the extension is a 32-char hex uuid
        let id_seg = name
            .strip_prefix("My.Video.rdlp-tmp-")
            .and_then(|s| s.strip_suffix(".mp4"))
            .expect("seam name shape");
        assert_eq!(
            id_seg.len(),
            32,
            "uuid simple form is 32 hex chars: {id_seg}"
        );
        assert!(
            id_seg.chars().all(|c| c.is_ascii_hexdigit()),
            "got: {id_seg}"
        );
        // round-trips back to the clean name via the Slice-1 strip helper
        assert_eq!(finalize_to_clean(&seam), Some(clean));
    }

    #[test]
    fn seam_path_unique_across_calls() {
        let clean = PathBuf::from("/v/X.mp4");
        assert_ne!(seam_path(&clean), seam_path(&clean));
    }

    #[test]
    fn seam_path_is_noop_for_stdout() {
        assert_eq!(seam_path(&PathBuf::from("-")), PathBuf::from("-"));
    }

    /// `finalize_part` must overwrite an existing clean file.
    ///
    /// This exercises the Windows-safe path: on POSIX `rename(2)` replaces
    /// atomically, but `tokio::fs::rename` on Windows fails if the destination
    /// already exists. For `process_local_file` with `keep_inputs=true`, the
    /// borrowed source file still exists on disk when the survivor is finalized
    /// to the same path (same-container in-place update), so `finalize_part`
    /// must remove it first (#414).
    #[tokio::test]
    #[allow(clippy::disallowed_methods)] // std::fs helpers in test fixtures — per clippy.toml policy (c)
    async fn test_finalize_part_overwrites_existing_clean() {
        use std::io::Write as _;
        use tempfile::TempDir;
        let dir = TempDir::new().expect("tempdir");

        // The survivor (temp-named output from the pipeline).
        let part = dir.path().join("video.rdlp-tmp-abc123.mp4");
        // The pre-existing clean destination (the borrowed source still on disk).
        let clean = dir.path().join("video.mp4");

        // Write distinct sentinel content so we can tell which file won.
        {
            let mut f = std::fs::File::create(&part).expect("create part");
            f.write_all(b"survivor-content").expect("write part");
        }
        {
            let mut f = std::fs::File::create(&clean).expect("create clean");
            f.write_all(b"old-source-content").expect("write clean");
        }

        assert!(part.exists(), "part must exist before finalize");
        assert!(clean.exists(), "clean must exist before finalize");

        finalize_part(&part, &clean)
            .await
            .expect("finalize_part must succeed even when clean already exists");

        // The survivor's content is now at the clean path.
        let content = std::fs::read(&clean).expect("read clean after finalize");
        assert_eq!(
            content, b"survivor-content",
            "clean must contain survivor content"
        );

        // The temp part must be gone.
        assert!(!part.exists(), "part must be removed after finalize");
    }
}
