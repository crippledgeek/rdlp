//! Resume detection and chunk merging functionality

use super::{Orchestrator, errors::Result};
use anyhow::Context;
use log::{debug, warn};
use rdlp_downloader::{ChunkKind, ChunkSet};
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Upper bound on how many distinct download attempts (`download_id`s) a
/// scan probes for new-style chunks. Not a grammar constraint — `download_id`
/// is a monotonic per-process counter starting at 0, so any in-progress or
/// recently-abandoned download is well within this window.
const MAX_DOWNLOAD_ID_SCAN: u64 = 100;

/// Ceiling on how many sequential chunk ids a scan probes within one chunk
/// set before concluding the set is exhausted.
///
/// This is a sanity ceiling against a pathological or corrupted directory,
/// not a live limit on any grammar: new-style power-of-two chunking (and the
/// resume grammar, which chunks the same way) can legitimately produce
/// thousands of small chunks, and — per #559's investigation — the legacy
/// grammar's historical `concurrent_fragments` cap of 10 was never itself
/// enforced as a scan/cleanup bound. A hardcoded `0..10` cleanup bound would
/// silently strand any legacy chunk set that ever did exceed 10 (see
/// `test_legacy_chunks_beyond_old_ten_chunk_bound_survive`); using the same
/// generous ceiling everywhere removes that trap.
///
/// Only [`collect_contiguous_chunks`] breaks on the first missing id within
/// this range — contiguity is a real correctness requirement there, since you
/// cannot merge across a hole. [`log_legacy_chunks`] and
/// [`log_orphaned_resume_chunks`] do NOT break on a hole: an interrupted
/// adaptive/resume download completes chunks out of order, so a holed set
/// (e.g. `resume0`, `resume2`, `resume4`) is the normal case there, and
/// breaking on the first hole would leak the remainder (#568 C1).
const CHUNK_SCAN_CEILING: u64 = 10_000;

/// Information about detected chunk files
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    /// Download ID (None for old-style chunks without ID)
    pub(crate) download_id: Option<u64>,
    /// Chunk file paths in order
    pub(crate) chunk_paths: Vec<PathBuf>,
    /// Total size of all chunks
    pub(crate) total_size: u64,
}

/// Probe `set` for a contiguous run of chunk ids starting at 0, stopping at
/// the first missing id (or at [`CHUNK_SCAN_CEILING`]). Read-only: builds
/// each candidate path via [`ChunkSet::path_in`] and checks existence, never
/// enumerating the directory's contents.
///
/// No chunk-0 sentinel is needed here (unlike [`log_legacy_chunks`] and
/// [`log_orphaned_resume_chunks`]): breaking on the first missing id is
/// already the cheapest possible short-circuit for the "nothing here" case
/// — an absent chunk 0 is itself the first miss, so the loop below exits
/// after exactly one probe.
async fn collect_contiguous_chunks(set: &ChunkSet, parent_dir: &Path) -> (Vec<PathBuf>, u64) {
    let mut chunk_paths = Vec::new();
    let mut total_size = 0u64;

    for chunk_id in 0..CHUNK_SCAN_CEILING {
        let chunk_path = set.path_in(parent_dir, chunk_id);
        // A single async `metadata` call replaces the previous synchronous
        // `exists()` pre-check plus a second async `metadata` call — same
        // information, half the syscalls, and no blocking call on the async
        // executor thread (#568 C4).
        match tokio::fs::metadata(&chunk_path).await {
            Ok(metadata) => {
                total_size += metadata.len();
                chunk_paths.push(chunk_path);
            }
            Err(_) => break,
        }
    }

    (chunk_paths, total_size)
}

/// Detect chunk files for a given output path
///
/// Supports both:
/// - Old-style: `{filename}.part{i}` (Phase 2 coarse-grained chunking)
/// - New-style: `{filename}.{downloadid}.part{i}` (Phase 2.5+ power-of-two chunking)
///
/// Returns the most recent chunk set (highest download ID)
async fn detect_chunk_files(output_path: &Path) -> Option<ChunkInfo> {
    // Validates the precondition every `ChunkSet` constructor below shares.
    output_path.file_name()?;
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut all_chunks: Vec<ChunkInfo> = Vec::new();

    // 1. Check for new-style chunks: {filename}.{downloadid}.part{i}
    for download_id in 0..MAX_DOWNLOAD_ID_SCAN {
        // Precondition already validated above; a failure here can only mean
        // `output_path` changed underneath us mid-scan, so skip rather than panic.
        let Ok(set) = ChunkSet::for_attempt(output_path, download_id, ChunkKind::Fresh) else {
            break;
        };
        let (chunk_paths, total_size) = collect_contiguous_chunks(&set, parent_dir).await;

        if !chunk_paths.is_empty() {
            all_chunks.push(ChunkInfo {
                download_id: Some(download_id),
                chunk_paths,
                total_size,
            });
        }
    }

    // 2. Check for old-style (legacy) chunks: {filename}.part{i}
    // Precondition already validated above, so `Err` can only mean `output_path`
    // changed underneath us mid-scan: skip the legacy grammar and report whatever
    // the new-style scan already found, rather than duplicating the reduction.
    match ChunkSet::legacy(output_path) {
        Ok(legacy_set) => {
            let (old_chunk_paths, old_total_size) =
                collect_contiguous_chunks(&legacy_set, parent_dir).await;

            if !old_chunk_paths.is_empty() {
                all_chunks.push(ChunkInfo {
                    download_id: None,
                    chunk_paths: old_chunk_paths,
                    total_size: old_total_size,
                });
            }
        }
        Err(_) => debug!("Skipping legacy chunk scan: output path lost its filename mid-scan"),
    }

    // 3. Return the most recent chunk set (highest download ID)
    // Priority: new-style chunks with highest ID > old-style chunks
    all_chunks.into_iter().max_by_key(|info| {
        // Sort key: (has_download_id, download_id_value)
        // This ensures new-style chunks come before old-style, and higher IDs come first
        (info.download_id.is_some(), info.download_id.unwrap_or(0))
    })
}

/// Merge chunk files into the output file
///
/// Supports both old-style and new-style chunk patterns
///
/// # Ownership (#573)
///
/// This is the one place in this module that still deletes on a bare probed
/// path, and it is safe to: it deletes only the chunks it has itself just
/// copied into `output_path`, from the set `detect_chunk_files` selected for
/// this exact merge — not an independently-probed leftover. Once #675 lands,
/// that selection is additionally verified against the run's own manifest
/// (`download_id` + `kind` + per-chunk length), which is the ownership proof
/// [`log_legacy_chunks`] and [`log_orphaned_resume_chunks`] cannot obtain for
/// an orphaned set found lying around.
///
/// Deletes what it copied, after the merged output is flushed, so a failed
/// merge leaves every chunk in place (#573 follow-up): copying a chunk and
/// deleting it immediately, before `flush()`, meant a failure on chunk N
/// (open/copy, or the flush itself) had already unlinked chunks `0..N` while
/// their bytes existed only in a truncated, unflushed output — data loss on
/// the very path meant to be recoverable.
pub async fn merge_chunk_files(output_path: &Path, chunk_info: &ChunkInfo) -> anyhow::Result<u64> {
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufWriter};

    let chunk_count = chunk_info.chunk_paths.len();
    let chunk_type = chunk_info.download_id.map_or_else(
        || "old-style".to_owned(),
        |id| format!("new-style (ID: {id})"),
    );

    debug!(chunk_count, chunk_type:?; "Merging chunks");

    // Create output file
    let file = File::create(output_path)
        .await
        .with_context(|| format!("failed to create output file {}", output_path.display()))?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file); // 2 MB buffer

    let mut total_size = 0u64;
    let mut copied_paths = Vec::with_capacity(chunk_count);

    // Merge each chunk in order — no deletion here; a chunk is only ever
    // removed once every chunk has been copied AND the output is flushed.
    for (idx, chunk_path) in chunk_info.chunk_paths.iter().enumerate() {
        if !chunk_path.exists() {
            anyhow::bail!("missing chunk file: {}", chunk_path.display());
        }

        let mut chunk_file = File::open(chunk_path)
            .await
            .with_context(|| format!("failed to open chunk file {}", chunk_path.display()))?;

        let bytes_copied = tokio::io::copy(&mut chunk_file, &mut writer)
            .await
            .with_context(|| format!("failed to copy chunk {} to output", chunk_path.display()))?;

        total_size += bytes_copied;
        copied_paths.push(chunk_path.clone());

        // Progress update every 100 chunks
        if (idx + 1) % 100 == 0 || idx == chunk_count - 1 {
            debug!(merged = idx + 1, total = chunk_count; "   Merge progress");
        }
    }

    writer
        .flush()
        .await
        .context("failed to flush merged output file")?;

    // The merge has succeeded and the output is durable on disk from this
    // process's point of view — now, and only now, remove the chunks it
    // consumed. A delete failure here does not undo a successful merge, so
    // it is logged rather than propagated as an error.
    let mut left_behind = 0usize;
    for chunk_path in &copied_paths {
        if let Err(e) = tokio::fs::remove_file(chunk_path).await {
            left_behind += 1;
            warn!(
                "Failed to remove merged chunk file {} (the merged output is complete; \
                 remove it by hand): {e}",
                chunk_path.display()
            );
        }
    }

    if left_behind == 0 {
        debug!(chunk_count; "Cleaned up chunk files");
    } else {
        debug!(chunk_count, left_behind; "Merged chunk files; some could not be removed");
    }

    Ok(total_size)
}

/// Probe `set` for the chunk ids present under `parent_dir`: a chunk-0
/// sentinel check first, and — only if chunk 0 exists — a hole-tolerant scan
/// up to [`CHUNK_SCAN_CEILING`] that never breaks on a missing id (an
/// interrupted download completes chunks out of order, so a holed set is the
/// normal case — see the module docs above). Read-only.
///
/// Shared by [`log_legacy_chunks`] and [`log_orphaned_resume_chunks`]
/// (#573): both grammars need the identical sentinel-gated, hole-tolerant
/// discovery — the two callers differ only in what they log about what they
/// found (a legacy set has no `download_id` to report; a resume set does),
/// which is why the logging stays with each caller rather than moving here.
async fn scan_chunk_set(set: &ChunkSet, parent_dir: &Path) -> Vec<PathBuf> {
    let sentinel = set.path_in(parent_dir, 0);
    if tokio::fs::metadata(&sentinel).await.is_err() {
        return Vec::new();
    }

    let mut found = vec![sentinel];
    for chunk_id in 1..CHUNK_SCAN_CEILING {
        let chunk_path = set.path_in(parent_dir, chunk_id);
        if tokio::fs::metadata(&chunk_path).await.is_ok() {
            found.push(chunk_path);
        }
    }
    found
}

/// Discover legacy-grammar chunks (`{filename}.part{i}`) left behind when
/// they are no longer going to be used for resume (a complete/oversized/
/// simple-partial file was found, or new-style chunks were used instead),
/// and log them for the operator to remove manually. **Never deletes**
/// (#573).
///
/// # Why this only logs, and never deletes
///
/// The legacy grammar carries no `download_id` — it predates power-of-two
/// chunking, back when only one download could be in flight against a given
/// output path at a time. That assumption no longer holds: a same-named
/// `.partN` file next to today's output path may belong to a second,
/// concurrently-running rdlp process (the same reasoning
/// [`log_orphaned_resume_chunks`] applies to the resume grammar, and the
/// same defect class #558 was about, one crate over). A probed path proves
/// only that a file exists there, never who is writing it. Separately, no
/// shipping writer produces this grammar any more (see the module docs), so
/// at most this is an old build's leftover — worth surfacing to the
/// operator, not worth risking a live peer's file to reclaim.
///
/// Deletion of a chunk set stays legitimate exactly where ownership can be
/// proven: [`merge_chunk_files`] deletes only the chunks it has just
/// consumed into the output it created.
///
/// # Sentinel gate and hole tolerance
///
/// Delegates the scan to [`scan_chunk_set`] (see its docs for the sentinel
/// short-circuit and hole-tolerance rationale, shared verbatim with
/// [`log_orphaned_resume_chunks`]). This runs on essentially every
/// `detect_resume_point` call (up to 3x per call — see the branches below),
/// so bounding the common "nothing here" case to a single syscall matters.
///
/// Narrow limitation, shared with `log_orphaned_resume_chunks`: a set that
/// has lost chunk 0 specifically (to some other partial cleanup) is not
/// discovered by this pass.
///
/// Returns every discovered chunk path, so callers/tests can assert on
/// discovery independently of the (absent) deletion side effect.
pub async fn log_legacy_chunks(output_path: &Path) -> Vec<PathBuf> {
    let Ok(set) = ChunkSet::legacy(output_path) else {
        return Vec::new();
    };
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
    let found = scan_chunk_set(&set, parent_dir).await;

    if !found.is_empty() {
        warn!(
            chunk_count = found.len();
            "Found {} legacy-style chunk file(s) next to {}: not deleting \
             automatically — the legacy grammar carries no download_id, so a \
             same-named file may belong to another rdlp process, and no \
             shipping writer produces this grammar any more, so at most \
             this is an old build's leftover. Remove '{}.part*' manually \
             once you've confirmed no other rdlp process is using this \
             output path.",
            found.len(),
            output_path.display(),
            output_path.display(),
        );
        for path in &found {
            debug!("  Found legacy chunk: {}", path.display());
        }
    }

    found
}

/// Discover orphaned resume-kind chunk sets (`{filename}.{download_id}.resume{i}`)
/// left behind by an abandoned resume attempt, and log them for the operator
/// to remove manually (refs #568, #559, #571 HIGH). **Never deletes.**
///
/// A resume chunk set encodes only a chunk index *relative* to the resume
/// attempt's byte offset — the offset itself is never persisted in the file
/// name or anywhere on disk (see `Attempt::Resume` in
/// `rdlp_downloader::http::parallel`), so a resume chunk set discovered here
/// can never be safely merged: there is no way to recover which byte offset
/// its `chunk_id 0` continues from, and the base file it was meant to extend
/// may since have been deleted (an oversized-file restart) or already
/// resumed independently at a different offset.
///
/// # Why this only logs, and never deletes
///
/// `download_id` is a monotonic counter **per process**, starting at 0. Two
/// rdlp processes downloading to the same output path concurrently are
/// therefore using the same low ids *at the same time* — an existence probe
/// here cannot tell "abandoned by a past attempt" apart from "being written
/// right now by a live peer" (the exact defect class #558 was about, one
/// crate over). `TempRegistry::cleanup_stale` in `rdlp-postprocess`
/// (`pipeline::registry`) earns the right to delete by requiring an fs4
/// exclusive advisory lock on a sidecar that its own writer
/// (`FileTracker::register`) takes at creation time. The downloader's chunk
/// writer takes no equivalent lock on chunk files, so reproducing that proof
/// here would require a cross-crate change to `rdlp-downloader`'s write
/// path — a real option, but out of proportion to the benefit: an orphaned
/// resume set can never be merged anyway (see above), so all automatic
/// cleanup would reclaim is a few megabytes of disk, and #568's acceptance
/// criterion 4 explicitly allows "documented as requiring manual cleanup" as
/// an alternative to automatic reclamation. This function takes that
/// alternative: discover, log with the exact paths implied by
/// `download_id`, and leave removal to the operator.
///
/// # Hole tolerance
///
/// Never breaks a per-`download_id` scan on the first missing chunk id:
/// resume chunks complete out of order on the adaptive path (`try_buffer_unordered`),
/// so a holed set (e.g. `resume0`, `resume2`, `resume4`) is the NORMAL case,
/// not evidence the set ends there (#568 C1). Chunk id 0 is checked first as
/// a cheap sentinel — a resume attempt always schedules id 0 immediately, so
/// its absence means this `download_id` was never used, and the rest of the
/// ceiling is skipped for it. This bounds the cost of the overwhelmingly
/// common "nothing orphaned" case to one probe per `download_id`. Narrow
/// limitation: a set that has lost chunk 0 specifically (e.g. to some other
/// partial cleanup) is not discovered by this pass.
///
/// Returns every discovered chunk path (across all `download_id`s), so
/// callers/tests can assert on discovery independently of the (absent)
/// deletion side effect.
pub async fn log_orphaned_resume_chunks(output_path: &Path) -> Vec<PathBuf> {
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));
    let mut discovered = Vec::new();

    for download_id in 0..MAX_DOWNLOAD_ID_SCAN {
        let Ok(set) = ChunkSet::for_attempt(output_path, download_id, ChunkKind::Resume) else {
            break;
        };

        let found_for_id = scan_chunk_set(&set, parent_dir).await;
        if found_for_id.is_empty() {
            continue;
        }

        warn!(
            download_id,
            chunk_count = found_for_id.len();
            "Found an orphaned resume chunk set (download_id {download_id}) next to {}: \
             not deleting automatically — a concurrent rdlp process may still own this \
             download_id, and safely merging would require the original byte offset, \
             which chunk file names don't encode. Remove \
             '{}.{download_id}.resume*' manually once you've confirmed no other rdlp \
             process is using this output path.",
            output_path.display(),
            output_path.display(),
        );
        discovered.extend(found_for_id);
    }

    discovered
}

impl Orchestrator {
    /// Detect the resume point for a download
    ///
    /// Checks for:
    /// 1. Existing complete or partial download file
    /// 2. Interrupted parallel download chunks:
    ///    - New-style (Phase 2.5+): `{filename}.{downloadid}.part{i}`
    ///    - Old-style (Phase 2): `{filename}.part{i}`
    ///
    /// Prioritizes new-style chunks (highest download ID) over old-style.
    /// Automatically merges the chosen chunk set (deleting only the chunks
    /// it consumed) and logs — but never deletes — any other chunk set
    /// found alongside it; a probed path proves existence, never ownership
    /// (#573).
    ///
    /// Returns the byte offset to resume from (0 for fresh download)
    #[instrument(skip(self), fields(path = %output_path.display()))]
    pub(super) async fn detect_resume_point(
        &self,
        output_path: &Path,
        expected_size: Option<u64>,
    ) -> Result<u64> {
        // 0. Discover (and log-only) any orphaned resume-kind chunk sets.
        // Unconditional and first, so every branch below sees it run — not
        // just the branches that also happen to run legacy discovery (#568
        // C3: the old call was nested inside `cleanup_old_chunks`/one `else`
        // arm, so the legacy-chunk-present branch never reached it).
        log_orphaned_resume_chunks(output_path).await;

        // 1. Check for existing complete or partial download file
        if output_path.exists()
            && let Ok(metadata) = tokio::fs::metadata(output_path).await
        {
            let size = metadata.len();
            if size > 0 {
                // Check if file is already complete
                if let Some(expected) = expected_size {
                    if size == expected {
                        #[allow(clippy::cast_precision_loss)] // display-only MB value
                        let mb = size as f64 / (1024.0 * 1024.0);
                        debug!("File already downloaded ({mb:.1} MB), skipping...");
                        // Discover (log-only) any orphaned legacy chunks
                        log_legacy_chunks(output_path).await;
                        return Ok(size);
                    } else if size > expected {
                        #[allow(clippy::cast_precision_loss)] // display-only MB values
                        let (size_mb, exp_mb) = (
                            size as f64 / (1024.0 * 1024.0),
                            expected as f64 / (1024.0 * 1024.0),
                        );
                        warn!(
                            "Partial file is larger than expected ({size_mb:.1} MB > {exp_mb:.1} MB), starting fresh..."
                        );
                        tokio::fs::remove_file(output_path).await.ok();
                        log_legacy_chunks(output_path).await;
                        return Ok(0);
                    }
                }
                #[allow(clippy::cast_precision_loss)] // display-only MB value
                let size_mb = size as f64 / (1024.0 * 1024.0);
                debug!("Found partial download ({size_mb:.1} MB), resuming...");
                // Discover (log-only) any orphaned chunks from failed parallel attempts
                log_legacy_chunks(output_path).await;
                return Ok(size);
            }
        }

        // 2. Check for interrupted parallel download chunks
        if let Some(chunk_info) = detect_chunk_files(output_path).await {
            let chunk_type = chunk_info.download_id.map_or_else(
                || "old-style".to_owned(),
                |id| format!("new-style (download ID: {id})"),
            );

            #[allow(clippy::cast_precision_loss)] // display-only MB value
            let total_mb = chunk_info.total_size as f64 / (1024.0 * 1024.0);
            debug!(
                "Found {} interrupted {} chunk files ({total_mb:.1} MB), merging and resuming...",
                chunk_info.chunk_paths.len(),
                chunk_type,
            );

            // If using new-style chunks, discover (log-only) any old-style
            // chunks left alongside them
            if chunk_info.download_id.is_some() {
                log_legacy_chunks(output_path).await;
            }

            // Merge chunks into the main file
            match merge_chunk_files(output_path, &chunk_info).await {
                Ok(size) => {
                    #[allow(clippy::cast_precision_loss)] // display-only MB value
                    let mb = size as f64 / (1024.0 * 1024.0);
                    debug!(
                        chunks = chunk_info.chunk_paths.len(),
                        mb:?;
                        "Merged chunks into main file"
                    );
                    Ok(size)
                }
                Err(e) => {
                    // #573: a chunk this pass had not yet consumed carries no
                    // stronger ownership proof than having just been probed
                    // once — a set that failed to merge fails the same way
                    // on retry, so deleting it buys nothing and risks a
                    // concurrent writer's file. Log, never delete.
                    //
                    // `merge_chunk_files` only deletes a chunk after the
                    // merged output is flushed, so on failure every chunk it
                    // was given should still be on disk — but stat each
                    // rather than trusting the full `chunk_paths` list, since
                    // that's a probe, not a guarantee (e.g. a concurrent
                    // process racing the same paths). Whether `output_path`
                    // itself was left behind partially written depends on
                    // *where* the merge failed (nothing was written if
                    // `File::create` itself failed), so check rather than
                    // assert it.
                    let mut remaining = Vec::new();
                    for chunk_path in &chunk_info.chunk_paths {
                        match tokio::fs::metadata(chunk_path).await {
                            Ok(_) => remaining.push(chunk_path.clone()),
                            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                            Err(err) => {
                                debug!("Failed to stat chunk {}: {err}", chunk_path.display());
                            }
                        }
                    }
                    let output_note = if output_path.exists() {
                        format!(
                            "any partially written {} was left in place; ",
                            output_path.display()
                        )
                    } else {
                        String::new()
                    };
                    warn!(
                        chunk_count = remaining.len();
                        "Failed to merge chunks: {e}. Starting fresh; {output_note}not \
                         deleting the {} remaining chunk file(s).",
                        remaining.len(),
                    );
                    for chunk_path in &remaining {
                        debug!("  Leaving chunk file in place: {}", chunk_path.display());
                    }
                    Ok(0)
                }
            }
        } else {
            // No main file, and no legacy/new-style Fresh chunk set found
            // either. Orphaned resume chunks (if any) were already
            // discovered and logged by step 0 above.
            Ok(0)
        }
    }
}
