//! Playlist resume detection and subtitle retry logic
//!
//! Contains filesystem scanning for existing downloads, subtitle file
//! detection with fuzzy matching, and subtitle-only retry downloads.

use super::{
    HashMap, Orchestrator, PathBuf, RESUME_SUB_FORMATS, ResumeDetection, debug, info, warn,
};

/// Check if any subtitle file exists for the given language (exact or fuzzy).
///
/// Checks `{stem}.{lang}.{ext}` for all subtitle formats. If no exact match,
/// scans `{stem}.*.{ext}` patterns for fuzzy prefix matches (e.g., `"en"` finds
/// `title.English.vtt`).
///
/// `files` is the pre-scanned list of files in the playlist directory. Passing
/// this in rather than re-scanning the directory on every call avoids an
/// `O(episodes × langs)` blocking `std::fs::read_dir` walk on the async
/// runtime thread.
pub(super) fn has_subtitle_file(files: &[PathBuf], stem: &str, lang: &str) -> bool {
    // Exact match: {stem}.{lang}.{ext}
    let exact_names: Vec<String> = RESUME_SUB_FORMATS
        .iter()
        .map(|fmt| format!("{stem}.{lang}.{}", fmt.as_ext()))
        .collect();
    for file in files {
        if let Some(name) = file.file_name().and_then(|n| n.to_str())
            && exact_names.iter().any(|e| e == name)
        {
            return true;
        }
    }

    // Fuzzy match: scan for {stem}.*.{ext} and prefix-match the middle segment
    if lang.len() <= 3 {
        let lang_lower = lang.to_ascii_lowercase();
        for file in files {
            let Some(name_str) = file.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Must start with "{stem}." and have at least one more dot
            if let Some(rest) = name_str
                .strip_prefix(stem)
                .and_then(|r| r.strip_prefix('.'))
            {
                // rest = "English.vtt" -> split into ("English", "vtt")
                if let Some((mid, ext)) = rest.rsplit_once('.') {
                    let is_sub_ext = RESUME_SUB_FORMATS
                        .iter()
                        .any(|f| f.as_ext().eq_ignore_ascii_case(ext));
                    if is_sub_ext && mid.to_ascii_lowercase().starts_with(&lang_lower) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

impl Orchestrator {
    /// Detect existing files in playlist folder that match video titles
    ///
    /// Returns a [`ResumeDetection`] containing:
    /// - `completed`: sanitized title -> file path for completed downloads
    /// - `partial_count`: videos with leftover segment files
    /// - `missing_subs`: completed videos missing expected subtitle files
    ///
    /// # Note on .part files
    ///
    /// The `.part{i}` grammar checked below (`ChunkSet` in the non-playlist
    /// `orchestrator::resume` module) belongs only to HTTP parallel-chunk
    /// downloads (`filename.mp4.part0`, few large files, resumable). HLS has no `.part`
    /// naming at all: segments stream directly to the output file and resume
    /// state lives in `{output}.hls_state.json`, not in loose segment files
    /// (DASH's equivalent is a `.parts` *directory*, plural, also unrelated to
    /// this grammar). A `.part` match here is always leftover HTTP chunks.
    ///
    /// Since leftover chunk files are cleaned up before each download, we just
    /// report the count for user information.
    pub(super) async fn detect_existing_playlist_files(
        &self,
        playlist_dir: &std::path::Path,
        infos: &[rdlp_types::InfoDict],
        subtitle_langs: &[String],
        strict_subs: bool,
    ) -> ResumeDetection {
        let mut completed = HashMap::new();
        let mut partial_count = 0;
        let mut missing_subs = HashMap::new();

        if !playlist_dir.exists() {
            return ResumeDetection {
                completed,
                partial_count,
                missing_subs,
            };
        }

        // Get all files in the playlist directory
        let Ok(mut dir_entries) = tokio::fs::read_dir(playlist_dir).await else {
            return ResumeDetection {
                completed,
                partial_count,
                missing_subs,
            };
        };

        let mut files: Vec<PathBuf> = Vec::new();
        while let Ok(Some(entry)) = dir_entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            }
        }

        // Check each video in the playlist
        for info in infos {
            let sanitized_title = Self::sanitize_filename(&info.title);

            // Look for completed file matching this title
            let mut found_complete = false;
            let mut found_partial = false;

            for file_path in &files {
                let filename = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");

                // Check for .part files (HLS segments or HTTP chunks)
                if filename.starts_with(&sanitized_title) && filename.contains(".part") {
                    found_partial = true;
                    continue;
                }

                // Check for completed file
                if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str())
                    && file_stem == sanitized_title
                {
                    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

                    // Skip .part files
                    if ext.contains("part") {
                        found_partial = true;
                        continue;
                    }

                    // Reject .ts files -- HLS remux didn't complete
                    if ext.eq_ignore_ascii_case("ts") {
                        debug!(
                            file:% = filename;
                            "Skipping .ts file (remux incomplete)"
                        );
                        found_partial = true;
                        continue;
                    }

                    // Check if file has reasonable size (> 1MB)
                    if let Ok(metadata) = file_path.metadata() {
                        if metadata.len() > 1_000_000 {
                            // Check subtitle file existence. Use the already-
                            // collected `files` list from the outer async scan
                            // to avoid re-entering std::fs::read_dir once per
                            // (episode × lang) pair.
                            let subs_missing = if subtitle_langs.is_empty() {
                                false
                            } else {
                                !subtitle_langs
                                    .iter()
                                    .any(|lang| has_subtitle_file(&files, &sanitized_title, lang))
                            };

                            // Strict mode: missing subs invalidate the video
                            if strict_subs && subs_missing {
                                debug!(
                                    file:% = filename;
                                    "Skipping: missing subtitle file (strict mode)"
                                );
                                found_partial = true;
                                continue;
                            }

                            // Lenient mode: track for reporting / --retry-subs
                            if subs_missing {
                                missing_subs.insert(sanitized_title.clone(), file_path.clone());
                            }

                            completed.insert(sanitized_title.clone(), file_path.clone());
                            found_complete = true;
                            break;
                        }
                        // File exists but is too small — treat as partial
                        found_partial = true;
                    }
                }
            }

            if !found_complete && found_partial {
                partial_count += 1;
            }
        }

        ResumeDetection {
            completed,
            partial_count,
            missing_subs,
        }
    }

    /// Retry subtitle downloads for completed videos that are missing subs.
    ///
    /// Re-extracts fresh metadata for each episode and downloads subtitles
    /// using the structured pipeline. Returns the count of episodes that
    /// had subtitles successfully recovered.
    pub(super) async fn retry_missing_subtitles(
        &self,
        missing_subs: &HashMap<String, PathBuf>,
        infos: &[rdlp_types::InfoDict],
        sub_langs: &[String],
    ) -> usize {
        let count = missing_subs.len();
        info!("Retrying subtitle download for {count} episode(s) with missing subs");

        let mut recovered = 0usize;
        for (sanitized_title, video_path) in missing_subs {
            // Find the matching InfoDict by sanitized title
            let Some(ep_info) = infos
                .iter()
                .find(|i| Self::sanitize_filename(&i.title) == *sanitized_title)
            else {
                warn!(
                    title:% = sanitized_title;
                    "Could not find episode metadata for subtitle retry"
                );
                continue;
            };

            // Re-extract fresh metadata to get fresh subtitle URLs
            let fresh_info = match self.extract_lazy_formats(&ep_info.webpage_url).await {
                Ok(mut resolved) => {
                    // Preserve episode metadata from original stub
                    resolved.id.clone_from(&ep_info.id);
                    resolved.title.clone_from(&ep_info.title);
                    resolved.playlist.clone_from(&ep_info.playlist);
                    resolved.playlist_title.clone_from(&ep_info.playlist_title);
                    resolved.playlist_index = ep_info.playlist_index;
                    resolved.playlist_count = ep_info.playlist_count;
                    resolved
                }
                Err(e) => {
                    warn!(
                        title:? = ep_info.title;
                        "Failed to re-extract for subtitle retry: {e}"
                    );
                    continue;
                }
            };

            // Resolve per-episode subtitle URLs from language names
            let episode_subs = self.resolve_subtitles_for_episode(&fresh_info, sub_langs);

            let downloaded_subs = match self
                .download_subtitles(&fresh_info, video_path, &episode_subs)
                .await
            {
                Ok(subs) => subs,
                Err(e) => {
                    warn!(
                        title:? = ep_info.title;
                        "Subtitle retry download failed: {e}"
                    );
                    continue;
                }
            };

            if downloaded_subs.is_empty() {
                warn!(
                    title:? = ep_info.title;
                    "Subtitle retry: no subtitles downloaded"
                );
            } else {
                info!(
                    title:? = ep_info.title,
                    count = downloaded_subs.len();
                    "Subtitle retry succeeded"
                );
                recovered += 1;
            }
        }

        recovered
    }

    /// Log per-episode missing subtitle details.
    ///
    /// Uses `sanitize_filename` to match infos against `missing_subs` keys.
    pub(super) fn log_missing_subs(
        missing_subs: &HashMap<String, PathBuf>,
        infos: &[rdlp_types::InfoDict],
        total: usize,
    ) {
        infos
            .iter()
            .filter(|info| missing_subs.contains_key(&Self::sanitize_filename(&info.title)))
            .for_each(|info| {
                let pos = info.playlist_index.unwrap_or(0);
                warn!("  [{pos}/{total}] {}: no subtitle files found", info.title);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression guard for Phase 1 async audit Finding 3.6 (spec §5.1.3).
    //
    // Before the fix, `has_subtitle_file` performed an `std::fs::read_dir`
    // for every (episode × lang) pair in the fuzzy-match branch, on the
    // async runtime thread. After the fix, the directory is scanned exactly
    // once (via `tokio::fs::read_dir` in the enclosing async fn) and this
    // helper takes the pre-collected `&[PathBuf]`.
    //
    // This test locks in the new in-memory signature: the helper must
    // resolve matches purely from the slice, with no filesystem access.
    // Using a non-existent `/nonexistent` directory in the paths would make
    // a filesystem-reading implementation return false; the in-memory
    // implementation returns true.

    #[test]
    fn has_subtitle_file_exact_match_from_slice() {
        let files = vec![
            PathBuf::from("/nonexistent/video.mp4"),
            PathBuf::from("/nonexistent/video.en.srt"),
        ];
        assert!(has_subtitle_file(&files, "video", "en"));
    }

    #[test]
    fn has_subtitle_file_fuzzy_match_from_slice() {
        let files = vec![
            PathBuf::from("/nonexistent/video.mp4"),
            PathBuf::from("/nonexistent/video.English.vtt"),
        ];
        assert!(has_subtitle_file(&files, "video", "en"));
    }

    #[test]
    fn has_subtitle_file_returns_false_when_missing() {
        let files = vec![
            PathBuf::from("/nonexistent/video.mp4"),
            PathBuf::from("/nonexistent/other.en.srt"),
        ];
        assert!(!has_subtitle_file(&files, "video", "en"));
    }

    #[test]
    fn has_subtitle_file_empty_slice_returns_false() {
        let files: Vec<PathBuf> = Vec::new();
        assert!(!has_subtitle_file(&files, "video", "en"));
    }
}
