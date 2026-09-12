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
/// `test_cleanup_legacy_chunks_beyond_old_ten_chunk_bound`); using the same
/// generous ceiling everywhere removes that trap.
///
/// Only [`collect_contiguous_chunks`] breaks on the first missing id within
/// this range — contiguity is a real correctness requirement there, since you
/// cannot merge across a hole. [`cleanup_old_chunks`] and
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
/// No chunk-0 sentinel is needed here (unlike [`cleanup_old_chunks`] and
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

    // Merge each chunk in order
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

        // Progress update every 100 chunks
        if (idx + 1) % 100 == 0 || idx == chunk_count - 1 {
            debug!(merged = idx + 1, total = chunk_count; "   Merge progress");
        }

        // Delete chunk file after successful merge
        tokio::fs::remove_file(chunk_path)
            .await
            .with_context(|| format!("failed to remove chunk file {}", chunk_path.display()))?;
    }

    writer
        .flush()
        .await
        .context("failed to flush merged output file")?;

    debug!(chunk_count; "Cleaned up chunk files");

    Ok(total_size)
}

/// Clean up legacy-grammar chunks (`{filename}.part{i}`) when they are no
/// longer going to be used for resume (a complete/oversized/simple-partial
/// file was found, or new-style chunks were used instead).
///
/// Deletes only exact paths computed via [`ChunkSet::path_in`] — never a
/// directory sweep (`scripts/check-no-dir-sweep-delete.sh`, #558).
///
/// # Sentinel gate
///
/// This runs on essentially every `resolve_resume` call (up to 3x per
/// call — see the branches below), so its cost has to be bounded for the
/// overwhelmingly common case where there is no legacy chunk set at all.
/// Chunk id 0 is checked first: a legacy download always writes id 0 first,
/// so its absence means there is nothing here to clean up, and the rest of
/// [`CHUNK_SCAN_CEILING`] is skipped entirely rather than attempting up to
/// 10,000 `remove_file` calls that all resolve to `NotFound`. This mirrors
/// the sentinel [`log_orphaned_resume_chunks`] already uses for the same
/// reason.
///
/// When chunk 0 IS present, the full ceiling is scanned without breaking on
/// a hole: an interrupted adaptive/resume download completes chunks out of
/// order, so a holed set (e.g. `part0`, `part2`, `part5`) is the normal case,
/// and breaking on the first gap would leak the remainder (#568 C1).
///
/// Narrow limitation, shared with `log_orphaned_resume_chunks`: a set that
/// has lost chunk 0 specifically (to some other partial cleanup) is not
/// cleaned by this pass. That trade is acceptable — it bounds the cost of
/// the common "nothing here" case to a single syscall, and a legacy chunk
/// set missing its first chunk is itself an unusual, already-degraded state.
async fn cleanup_old_chunks(output_path: &Path) {
    let Ok(set) = ChunkSet::legacy(output_path) else {
        return;
    };
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut deleted = 0;
    let sentinel = set.path_in(parent_dir, 0);
    match tokio::fs::remove_file(&sentinel).await {
        Ok(()) => deleted += 1,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => debug!("Failed to remove legacy chunk {}: {e}", sentinel.display()),
    }

    for chunk_id in 1..CHUNK_SCAN_CEILING {
        let chunk_path = set.path_in(parent_dir, chunk_id);
        // Attempt the delete directly instead of probing with a synchronous
        // `exists()` first: `NotFound` tells us exactly what the probe
        // would have, in one syscall instead of two, without blocking the
        // async executor thread (#568 C4). Never break on a miss: a legacy
        // set can be holed same as any other grammar, and the scan bound is
        // [`CHUNK_SCAN_CEILING`], not "first gap".
        match tokio::fs::remove_file(&chunk_path).await {
            Ok(()) => deleted += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => debug!(
                "Failed to remove legacy chunk {}: {e}",
                chunk_path.display()
            ),
        }
    }

    if deleted > 0 {
        debug!(deleted; "Cleaned up legacy-style chunk files");
    }
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

        let sentinel = set.path_in(parent_dir, 0);
        if tokio::fs::metadata(&sentinel).await.is_err() {
            continue;
        }

        let mut found_for_id = vec![sentinel];
        for chunk_id in 1..CHUNK_SCAN_CEILING {
            let chunk_path = set.path_in(parent_dir, chunk_id);
            if tokio::fs::metadata(&chunk_path).await.is_ok() {
                found_for_id.push(chunk_path);
            }
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

/// What [`resolve_resume`] will enact for an output path — the decision
/// alone, produced by [`plan_resume`] without touching the disk.
#[derive(Debug)]
pub(super) enum ResumePlan {
    /// The `.rdlp-part` file's on-disk size matches `expected_size` exactly.
    AlreadyComplete(u64),
    /// The `.rdlp-part` file is smaller than `expected_size` (or
    /// `expected_size` is unknown) — resume from this byte offset.
    Resume(u64),
    /// The `.rdlp-part` file is LARGER than the extractor-reported
    /// `expected_size`. Extractor sizes are unverified (#674 reports
    /// mismatches), so the on-disk bytes are the more trustworthy artifact:
    /// the resolved policy is to set the file aside for inspection rather
    /// than delete it (#561), never to trust the smaller reported figure.
    LargerThanReported { size: u64, expected: u64 },
    /// No main file (or an empty one), but an interrupted chunk set was
    /// found and can be merged.
    MergeChunks(ChunkInfo),
    /// No main file and no chunk set — nothing to resume from.
    Fresh,
}

/// Determine [`ResumePlan`] for `output_path` without mutating anything on
/// disk (#561: `detect_resume_point` mixed this decision with the
/// deletes/merges that acted on it, so a caller that only wanted the plan
/// had no way to get one without also triggering the side effects).
///
/// `log_orphaned_resume_chunks` is the one exception: it is itself a
/// log-only query (see its own doc comment) and is folded in here so every
/// caller of `plan_resume` still gets the discovery, not just callers of
/// [`resolve_resume`].
///
/// Mirrors the two-step shape `detect_resume_point` used to inline: check
/// the main file, then fall back to chunk detection.
pub(super) async fn plan_resume(output_path: &Path, expected_size: Option<u64>) -> ResumePlan {
    // Discover (and log-only) any orphaned resume-kind chunk sets. Runs
    // unconditionally so every plan_resume call sees it, matching the
    // previous unconditional placement in detect_resume_point (#568 C3).
    log_orphaned_resume_chunks(output_path).await;

    if output_path.exists()
        && let Ok(metadata) = tokio::fs::metadata(output_path).await
    {
        let size = metadata.len();
        if size > 0 {
            if let Some(expected) = expected_size {
                match size.cmp(&expected) {
                    std::cmp::Ordering::Equal => return ResumePlan::AlreadyComplete(size),
                    std::cmp::Ordering::Greater => {
                        return ResumePlan::LargerThanReported { size, expected };
                    }
                    std::cmp::Ordering::Less => {}
                }
            }
            return ResumePlan::Resume(size);
        }
    }

    detect_chunk_files(output_path)
        .await
        .map_or(ResumePlan::Fresh, ResumePlan::MergeChunks)
}

/// The enacted outcome of [`Orchestrator::resolve_resume`] — what a caller
/// should do next, converging the `offset == expected_size` /
/// `offset > 0` mapping that both `state/mod.rs` and `playlist/episode.rs`
/// previously duplicated (#561).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ResumeOutcome {
    /// The output is already complete at `size` bytes. The caller still
    /// owns finalizing `.rdlp-part` -> the clean output path, since only the
    /// caller knows that path.
    Complete { size: u64 },
    /// Resume the download from this byte offset.
    Resume(u64),
    /// Start fresh.
    Fresh,
}

#[cfg(test)]
impl ResumeOutcome {
    /// Test-only convenience: the byte count a caller would resume-or-finalize
    /// from, collapsing `Complete`/`Resume`/`Fresh` the way the old
    /// `detect_resume_point` u64 return did. Production code matches on the
    /// variant directly instead — this exists so the pre-existing chunk-merge
    /// tests (which only care about "how many bytes ended up on disk") don't
    /// need to be rewritten variant-by-variant.
    pub(super) fn size(&self) -> u64 {
        match self {
            Self::Complete { size } | Self::Resume(size) => *size,
            Self::Fresh => 0,
        }
    }
}

/// Decide whether `size` on-disk bytes represent a complete download or a
/// partial one to resume from.
///
/// The single place both the main-file check (`ResumePlan::AlreadyComplete`
/// / `Resume`, already split by `plan_resume`) and the post-merge check ask
/// this question, so the answer can't drift between them (#561 spec review:
/// the first cut of this split returned `Resume(size)` unconditionally after
/// a merge, so a chunk set whose recorded total equaled `expected_size` was
/// asked to resume from EOF instead of finalizing).
const fn outcome_for_size(size: u64, expected_size: Option<u64>) -> ResumeOutcome {
    match expected_size {
        Some(expected) if size == expected => ResumeOutcome::Complete { size },
        _ => ResumeOutcome::Resume(size),
    }
}

/// Move an oversized `.rdlp-part` file aside instead of deleting it (#561).
///
/// Extractor-reported sizes are unverified (#674 already reports
/// mismatches), so an on-disk file larger than the reported size is treated
/// as suspect metadata, not corrupt data — the bytes are kept for manual
/// inspection. Reuses the `.rdlp-bak-{uuid}` naming `finalize_part` uses for
/// its Windows backup path (see `naming::BAK_MARKER`): that name is already
/// proven invisible to `TempRegistry::cleanup_stale`'s `.rdlp-tmp-` marker
/// scan, which is the exact trap a fresh naming scheme here could reintroduce
/// — but that same invisibility means nothing ever automatically removes a
/// `.rdlp-bak-*` sidecar (see `paths.rs`'s `neutralize_temp_markers` doc
/// comment): it is a deliberately permanent artifact until a human clears it.
///
/// Every error propagates — a failed rename must not be swallowed into a
/// silent "proceed as Fresh" the way the old `.ok()` did, because that would
/// leave the caller believing the file is gone when it might still be in
/// place at `output_path` (#561).
async fn set_aside_oversized(output_path: &Path, size: u64, expected: u64) -> anyhow::Result<()> {
    let backup = super::naming::bak_sidecar_path(output_path);

    tokio::fs::rename(output_path, &backup)
        .await
        .with_context(|| {
            format!(
                "failed to set aside oversized partial download {} ({size} bytes > \
                 extractor-reported {expected} bytes) to {}",
                output_path.display(),
                backup.display()
            )
        })?;

    warn!(
        size,
        expected;
        "Partial file is larger than the extractor-reported size — extractor sizes are \
         unverified (#674), so the on-disk bytes are kept rather than deleted; set aside to {} \
         for manual inspection (nothing automatically removes it — clear it by hand once \
         inspected), starting fresh",
        backup.display(),
    );
    Ok(())
}

impl Orchestrator {
    /// Determine the resume plan for a download. Pure query — see
    /// [`plan_resume`]. Exposed on `Orchestrator` only so test/caller call
    /// sites read consistently with [`Self::resolve_resume`]; delegates
    /// entirely to the free function.
    #[cfg(test)]
    pub(super) async fn plan_resume(
        &self,
        output_path: &Path,
        expected_size: Option<u64>,
    ) -> ResumePlan {
        plan_resume(output_path, expected_size).await
    }

    /// Resolve the resume point for a download and enact the plan.
    ///
    /// Checks for:
    /// 1. Existing complete or partial download file
    /// 2. Interrupted parallel download chunks:
    ///    - New-style (Phase 2.5+): `{filename}.{downloadid}.part{i}`
    ///    - Old-style (Phase 2): `{filename}.part{i}`
    ///
    /// Prioritizes new-style chunks (highest download ID) over old-style.
    /// Automatically merges and cleans up chunk files.
    ///
    /// This is the single call both `state/mod.rs` and
    /// `playlist/episode.rs` use — previously each duplicated the
    /// `offset == expected_size` / `offset > 0` mapping over the raw u64
    /// `detect_resume_point` returned (#561); that mapping now lives once,
    /// in the match below.
    #[instrument(skip(self), fields(path = %output_path.display()))]
    pub(super) async fn resolve_resume(
        &self,
        output_path: &Path,
        expected_size: Option<u64>,
    ) -> Result<ResumeOutcome> {
        match plan_resume(output_path, expected_size).await {
            ResumePlan::AlreadyComplete(size) => {
                #[allow(clippy::cast_precision_loss)] // display-only MB value
                let mb = size as f64 / (1024.0 * 1024.0);
                debug!("File already downloaded ({mb:.1} MB), skipping...");
                // Best-effort: a legacy chunk set left over next to an
                // already-complete file is cosmetic clutter, not correctness
                // — worth cleaning up, but not worth failing the whole
                // resolve over (matches the pre-#561 behavior).
                cleanup_old_chunks(output_path).await;
                Ok(outcome_for_size(size, expected_size))
            }
            ResumePlan::Resume(size) => {
                #[allow(clippy::cast_precision_loss)] // display-only MB value
                let size_mb = size as f64 / (1024.0 * 1024.0);
                debug!("Found partial download ({size_mb:.1} MB), resuming...");
                cleanup_old_chunks(output_path).await;
                Ok(outcome_for_size(size, expected_size))
            }
            ResumePlan::LargerThanReported { size, expected } => {
                set_aside_oversized(output_path, size, expected).await?;
                cleanup_old_chunks(output_path).await;
                Ok(ResumeOutcome::Fresh)
            }
            ResumePlan::MergeChunks(chunk_info) => {
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

                // If using new-style chunks, clean up any old-style chunks first
                if chunk_info.download_id.is_some() {
                    cleanup_old_chunks(output_path).await;
                }

                match merge_chunk_files(output_path, &chunk_info).await {
                    Ok(size) => {
                        #[allow(clippy::cast_precision_loss)] // display-only MB value
                        let mb = size as f64 / (1024.0 * 1024.0);
                        debug!(
                            chunks = chunk_info.chunk_paths.len(),
                            mb:?;
                            "Merged chunks into main file"
                        );
                        // #561 spec review: a merged total that equals
                        // expected_size is complete, not "resume from EOF" —
                        // route through the same decision `AlreadyComplete`
                        // uses so the two paths can't drift.
                        Ok(outcome_for_size(size, expected_size))
                    }
                    Err(e) => {
                        warn!("Failed to merge chunks: {e}. Starting fresh.");
                        for chunk_path in &chunk_info.chunk_paths {
                            let _ = tokio::fs::remove_file(chunk_path).await;
                        }
                        Ok(ResumeOutcome::Fresh)
                    }
                }
            }
            ResumePlan::Fresh => {
                // No main file, and no legacy/new-style chunk set found
                // either. Orphaned resume chunks (if any) were already
                // discovered and logged inside plan_resume.
                Ok(ResumeOutcome::Fresh)
            }
        }
    }
}
