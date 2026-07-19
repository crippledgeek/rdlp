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
/// set before concluding the set is exhausted (the scan breaks earlier, on
/// the first missing chunk id, in the overwhelmingly common case).
///
/// This is a sanity ceiling against a pathological or corrupted directory,
/// not a live limit on either grammar: new-style power-of-two chunking can
/// legitimately produce thousands of small chunks, and — per #559's
/// investigation — the legacy grammar's historical `concurrent_fragments`
/// cap of 10 was never itself enforced as a scan/cleanup bound. A hardcoded
/// `0..10` cleanup bound would silently strand any legacy chunk set that
/// ever did exceed 10 (see `test_cleanup_legacy_chunks_beyond_old_ten_chunk_bound`);
/// using the same generous ceiling for both grammars removes that trap
/// without pretending the legacy count is normally this large.
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
async fn collect_contiguous_chunks(set: &ChunkSet, parent_dir: &Path) -> (Vec<PathBuf>, u64) {
    let mut chunk_paths = Vec::new();
    let mut total_size = 0u64;

    for chunk_id in 0..CHUNK_SCAN_CEILING {
        let chunk_path = set.path_in(parent_dir, chunk_id);
        if chunk_path.exists() {
            if let Ok(metadata) = tokio::fs::metadata(&chunk_path).await {
                chunk_paths.push(chunk_path);
                total_size += metadata.len();
            }
        } else {
            break;
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
async fn cleanup_old_chunks(output_path: &Path) {
    let Ok(set) = ChunkSet::legacy(output_path) else {
        return;
    };
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut deleted = 0;
    for chunk_id in 0..CHUNK_SCAN_CEILING {
        let chunk_path = set.path_in(parent_dir, chunk_id);
        if chunk_path.exists() && tokio::fs::remove_file(&chunk_path).await.is_ok() {
            deleted += 1;
        }
    }

    if deleted > 0 {
        debug!(deleted; "Cleaned up legacy-style chunk files");
    }

    cleanup_orphaned_resume_chunks(output_path).await;
}

/// Clean up orphaned resume-kind chunk sets (`{filename}.{download_id}.resume{i}`)
/// left behind by an abandoned resume attempt (refs #568, #559).
///
/// A resume chunk set encodes only a chunk index *relative* to the resume
/// attempt's byte offset — the offset itself is never persisted in the file
/// name or anywhere on disk (see `Attempt::Resume` in
/// `rdlp_downloader::http::parallel`), so a resume chunk set discovered here
/// can never be safely merged: there is no way to recover which byte offset
/// its `chunk_id 0` continues from, and the base file it was meant to extend
/// may since have been deleted (an oversized-file restart) or already
/// resumed independently at a different offset. Discover-and-delete is the
/// safe contract; the bytes are simply re-downloaded by whatever fresh
/// attempt follows.
///
/// Deletes only exact paths computed via [`ChunkSet::path_in`] — never a
/// directory sweep (`scripts/check-no-dir-sweep-delete.sh`, #558).
async fn cleanup_orphaned_resume_chunks(output_path: &Path) {
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut deleted = 0;
    for download_id in 0..MAX_DOWNLOAD_ID_SCAN {
        let Ok(set) = ChunkSet::for_attempt(output_path, download_id, ChunkKind::Resume) else {
            return;
        };

        let mut deleted_for_id = 0;
        for chunk_id in 0..CHUNK_SCAN_CEILING {
            let chunk_path = set.path_in(parent_dir, chunk_id);
            if chunk_path.exists() {
                if tokio::fs::remove_file(&chunk_path).await.is_ok() {
                    deleted_for_id += 1;
                }
            } else {
                break;
            }
        }

        if deleted_for_id > 0 {
            warn!(
                download_id, deleted_for_id;
                "Deleting orphaned resume chunk set: safely merging requires the \
                 original byte offset, which chunk file names don't encode"
            );
            deleted += deleted_for_id;
        }
    }

    if deleted > 0 {
        debug!(deleted; "Cleaned up orphaned resume-style chunk files");
    }
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
    /// Automatically merges and cleans up chunk files.
    ///
    /// Returns the byte offset to resume from (0 for fresh download)
    #[instrument(skip(self), fields(path = %output_path.display()))]
    pub(super) async fn detect_resume_point(
        &self,
        output_path: &Path,
        expected_size: Option<u64>,
    ) -> Result<u64> {
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
                        // Clean up any orphaned chunks
                        cleanup_old_chunks(output_path).await;
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
                        cleanup_old_chunks(output_path).await;
                        return Ok(0);
                    }
                }
                #[allow(clippy::cast_precision_loss)] // display-only MB value
                let size_mb = size as f64 / (1024.0 * 1024.0);
                debug!("Found partial download ({size_mb:.1} MB), resuming...");
                // Clean up any orphaned chunks from failed parallel attempts
                cleanup_old_chunks(output_path).await;
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

            // If using new-style chunks, clean up any old-style chunks first
            if chunk_info.download_id.is_some() {
                cleanup_old_chunks(output_path).await;
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
                    warn!("Failed to merge chunks: {e}. Starting fresh.");
                    // Clean up partial chunks
                    for chunk_path in &chunk_info.chunk_paths {
                        let _ = tokio::fs::remove_file(chunk_path).await;
                    }
                    Ok(0)
                }
            }
        } else {
            // No main file, and no legacy/new-style Fresh chunk set found
            // either. Still clean up any orphaned resume chunks so they
            // don't leak indefinitely (#568/#559 item 4) — nothing else on
            // this path would ever reach them.
            cleanup_orphaned_resume_chunks(output_path).await;
            Ok(0)
        }
    }
}
