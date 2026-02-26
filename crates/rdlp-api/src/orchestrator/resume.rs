//! Resume detection and chunk merging functionality

use super::{Orchestrator, errors::*};
use log::{debug, warn};
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Information about detected chunk files
#[derive(Debug, Clone)]
pub(crate) struct ChunkInfo {
    /// Download ID (None for old-style chunks without ID)
    pub(crate) download_id: Option<u64>,
    /// Chunk file paths in order
    pub(crate) chunk_paths: Vec<PathBuf>,
    /// Total size of all chunks
    pub(crate) total_size: u64,
}

/// Detect chunk files for a given output path
///
/// Supports both:
/// - Old-style: `{filename}.part{i}` (Phase 2 coarse-grained chunking)
/// - New-style: `{filename}.{downloadid}.part{i}` (Phase 2.5+ power-of-two chunking)
///
/// Returns the most recent chunk set (highest download ID)
async fn detect_chunk_files(output_path: &Path) -> Option<ChunkInfo> {
    let base_name = output_path.file_name()?.to_string_lossy();
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut all_chunks: Vec<ChunkInfo> = Vec::new();

    // 1. Check for new-style chunks: {filename}.{downloadid}.part{i}
    // Scan for download IDs from 0 to 100 (reasonable upper bound)
    for download_id in 0..100 {
        let mut chunk_paths = Vec::new();
        let mut total_size = 0u64;

        // Scan for chunks with this download ID
        // Check up to 10000 chunks (power-of-two chunking can create many small chunks)
        for chunk_id in 0..10000 {
            let chunk_path = parent_dir.join(format!("{base_name}.{download_id}.part{chunk_id}"));

            if chunk_path.exists() {
                if let Ok(metadata) = tokio::fs::metadata(&chunk_path).await {
                    chunk_paths.push(chunk_path);
                    total_size += metadata.len();
                }
            } else {
                // No more chunks for this download ID
                break;
            }
        }

        if !chunk_paths.is_empty() {
            all_chunks.push(ChunkInfo {
                download_id: Some(download_id),
                chunk_paths,
                total_size,
            });
        }
    }

    // 2. Check for old-style chunks: {filename}.part{i}
    let mut old_chunk_paths = Vec::new();
    let mut old_total_size = 0u64;

    for i in 0..10 {
        // Old system used max 10 chunks (concurrent_fragments capped at 10)
        let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));

        if chunk_path.exists() {
            if let Ok(metadata) = tokio::fs::metadata(&chunk_path).await {
                old_chunk_paths.push(chunk_path);
                old_total_size += metadata.len();
            }
        } else {
            // Old chunks are sequential
            break;
        }
    }

    if !old_chunk_paths.is_empty() {
        all_chunks.push(ChunkInfo {
            download_id: None,
            chunk_paths: old_chunk_paths,
            total_size: old_total_size,
        });
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
pub(crate) async fn merge_chunk_files(output_path: &Path, chunk_info: &ChunkInfo) -> Result<u64> {
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufWriter};

    let chunk_count = chunk_info.chunk_paths.len();
    let chunk_type = match chunk_info.download_id {
        Some(id) => format!("new-style (ID: {id})"),
        None => "old-style".to_string(),
    };

    debug!(chunk_count, chunk_type:?; "Merging chunks");

    // Create output file
    let file = File::create(output_path)
        .await
        .map_err(OrchestratorError::ChunkMergeFailed)?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file); // 2 MB buffer

    let mut total_size = 0u64;

    // Merge each chunk in order
    for (idx, chunk_path) in chunk_info.chunk_paths.iter().enumerate() {
        if !chunk_path.exists() {
            return Err(OrchestratorError::MissingChunk {
                path: chunk_path.clone(),
            });
        }

        let mut chunk_file = File::open(chunk_path)
            .await
            .map_err(OrchestratorError::ChunkMergeFailed)?;

        let bytes_copied = tokio::io::copy(&mut chunk_file, &mut writer)
            .await
            .map_err(OrchestratorError::ChunkMergeFailed)?;

        total_size += bytes_copied;

        // Progress update every 100 chunks
        if (idx + 1) % 100 == 0 || idx == chunk_count - 1 {
            debug!(merged = idx + 1, total = chunk_count; "   Merge progress");
        }

        // Delete chunk file after successful merge
        tokio::fs::remove_file(chunk_path)
            .await
            .map_err(OrchestratorError::ChunkMergeFailed)?;
    }

    writer
        .flush()
        .await
        .map_err(OrchestratorError::ChunkMergeFailed)?;

    debug!(chunk_count; "Cleaned up chunk files");

    Ok(total_size)
}

/// Clean up old-style chunks when new-style chunks are used
async fn cleanup_old_chunks(output_path: &Path) {
    let Some(name) = output_path.file_name() else {
        return;
    };
    let base_name = name.to_string_lossy();
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    let mut deleted = 0;
    for i in 0..10 {
        let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));
        if chunk_path.exists() && tokio::fs::remove_file(&chunk_path).await.is_ok() {
            deleted += 1;
        }
    }

    if deleted > 0 {
        debug!(deleted; "Cleaned up old-style chunk files");
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
                        debug!(
                            "File already downloaded ({:.1} MB), skipping...",
                            size as f64 / (1024.0 * 1024.0)
                        );
                        // Clean up any orphaned chunks
                        cleanup_old_chunks(output_path).await;
                        return Ok(size);
                    } else if size > expected {
                        warn!(
                            "Partial file is larger than expected ({:.1} MB > {:.1} MB), starting fresh...",
                            size as f64 / (1024.0 * 1024.0),
                            expected as f64 / (1024.0 * 1024.0)
                        );
                        tokio::fs::remove_file(output_path).await.ok();
                        cleanup_old_chunks(output_path).await;
                        return Ok(0);
                    }
                }
                debug!(
                    "Found partial download ({:.1} MB), resuming...",
                    size as f64 / (1024.0 * 1024.0)
                );
                // Clean up any orphaned chunks from failed parallel attempts
                cleanup_old_chunks(output_path).await;
                return Ok(size);
            }
        }

        // 2. Check for interrupted parallel download chunks
        if let Some(chunk_info) = detect_chunk_files(output_path).await {
            let chunk_type = if let Some(id) = chunk_info.download_id {
                format!("new-style (download ID: {id})")
            } else {
                "old-style".to_string()
            };

            debug!(
                "Found {} interrupted {} chunk files ({:.1} MB), merging and resuming...",
                chunk_info.chunk_paths.len(),
                chunk_type,
                chunk_info.total_size as f64 / (1024.0 * 1024.0)
            );

            // If using new-style chunks, clean up any old-style chunks first
            if chunk_info.download_id.is_some() {
                cleanup_old_chunks(output_path).await;
            }

            // Merge chunks into the main file
            match merge_chunk_files(output_path, &chunk_info).await {
                Ok(size) => {
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
            Ok(0)
        }
    }
}
