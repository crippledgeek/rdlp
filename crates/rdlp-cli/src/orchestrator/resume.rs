//! Resume detection and chunk merging functionality

use super::{errors::*, Orchestrator};
use std::path::Path;

/// Merge interrupted parallel download chunks into a single file
///
/// Returns the total size of the merged file
pub(crate) async fn merge_chunk_files(output_path: &Path, chunk_count: usize) -> Result<u64> {
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufWriter};

    let base_name = output_path.file_name().unwrap().to_string_lossy();
    let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

    // Create output file
    let file = File::create(output_path)
        .await
        .map_err(OrchestratorError::ChunkMergeFailed)?;
    let mut writer = BufWriter::with_capacity(2 * 1024 * 1024, file); // 2 MB buffer

    let mut total_size = 0u64;

    // Merge each chunk in order
    for i in 0..chunk_count {
        let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));

        if !chunk_path.exists() {
            return Err(OrchestratorError::MissingChunk { path: chunk_path });
        }

        let mut chunk_file = File::open(&chunk_path)
            .await
            .map_err(OrchestratorError::ChunkMergeFailed)?;

        let bytes_copied = tokio::io::copy(&mut chunk_file, &mut writer)
            .await
            .map_err(OrchestratorError::ChunkMergeFailed)?;

        total_size += bytes_copied;

        // Delete chunk file after successful merge
        tokio::fs::remove_file(&chunk_path)
            .await
            .map_err(OrchestratorError::ChunkMergeFailed)?;
    }

    writer
        .flush()
        .await
        .map_err(OrchestratorError::ChunkMergeFailed)?;

    Ok(total_size)
}

impl Orchestrator {
    /// Detect the resume point for a download
    ///
    /// Checks for:
    /// 1. Existing complete or partial download file
    /// 2. Interrupted parallel download chunks (.part0, .part1, etc.)
    ///
    /// Returns the byte offset to resume from (0 for fresh download)
    pub(super) async fn detect_resume_point(
        &self,
        output_path: &Path,
        expected_size: Option<u64>,
    ) -> Result<u64> {
        if output_path.exists() {
            if let Ok(metadata) = tokio::fs::metadata(output_path).await {
                let size = metadata.len();
                if size > 0 {
                    // Check if file is already complete
                    if let Some(expected) = expected_size {
                        if size == expected {
                            println!(
                                "✓ File already downloaded ({:.1} MB), skipping...",
                                size as f64 / (1024.0 * 1024.0)
                            );
                            return Ok(size);
                        } else if size > expected {
                            println!(
                                "⚠️  Partial file is larger than expected ({:.1} MB > {:.1} MB), starting fresh...",
                                size as f64 / (1024.0 * 1024.0),
                                expected as f64 / (1024.0 * 1024.0)
                            );
                            tokio::fs::remove_file(output_path).await.ok();
                            return Ok(0);
                        }
                    }
                    println!(
                        "📋 Found partial download ({:.1} MB), resuming...",
                        size as f64 / (1024.0 * 1024.0)
                    );
                    return Ok(size);
                }
            }
        }

        // Check for interrupted parallel download chunks (.part0, .part1, etc.)
        let base_name = output_path.file_name().unwrap().to_string_lossy();
        let parent_dir = output_path.parent().unwrap_or_else(|| Path::new("."));

        // Look for .part files
        let mut total_chunk_size = 0u64;
        let mut chunk_count = 0;

        for i in 0..10 {
            // Check up to 10 chunks (concurrent_fragments is capped at 10)
            let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));
            if chunk_path.exists() {
                if let Ok(metadata) = tokio::fs::metadata(&chunk_path).await {
                    total_chunk_size += metadata.len();
                    chunk_count += 1;
                }
            }
        }

        if chunk_count > 0 {
            println!(
                "📋 Found {} interrupted chunk files ({:.1} MB), merging and resuming...",
                chunk_count,
                total_chunk_size as f64 / (1024.0 * 1024.0)
            );

            // Merge chunks into the main file
            match merge_chunk_files(output_path, chunk_count).await {
                Ok(size) => {
                    println!(
                        "✓ Merged {} chunks into main file ({:.1} MB)",
                        chunk_count,
                        size as f64 / (1024.0 * 1024.0)
                    );
                    Ok(size)
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to merge chunks: {e}. Starting fresh.");
                    // Clean up partial chunks
                    for i in 0..chunk_count {
                        let chunk_path = parent_dir.join(format!("{base_name}.part{i}"));
                        let _ = tokio::fs::remove_file(&chunk_path).await;
                    }
                    Ok(0)
                }
            }
        } else {
            Ok(0)
        }
    }
}
