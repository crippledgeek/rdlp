//! Per-episode download logic for playlist downloads.
//!
//! Owns `download_from_info_to_dir`, which downloads an individual
//! episode with lazy format resolution and CDN retry.

use super::{
    DownloadPlan, DownloadResult, Duration, Instant, Orchestrator, OrchestratorError, PathBuf,
    Result, TOKEN_MAX_AGE, debug, extract_cdn_host, is_reextractable_error, warn,
};

impl Orchestrator {
    /// Download from pre-extracted `InfoDict` to a specific directory
    ///
    /// This method is used by playlist downloads to save files to the playlist folder.
    /// Uses the shared CDN fallback loop via `download_with_cdn_fallback()`.
    ///
    /// Supports lazy format resolution: if `info.formats` is empty and
    /// `info.webpage_url` is set, formats are resolved on-the-fly via
    /// `extract_video()`.
    ///
    /// On CDN failure (invalid M3U8, 403/503), re-extracts fresh URLs up to
    /// `MAX_EXTRACT_RETRIES` times with a delay between attempts. This mirrors
    /// yt-dlp's `--extractor-retries` behavior.
    ///
    /// # Arguments
    /// * `info` - Pre-extracted video metadata
    /// * `interactive` - Whether interactive mode is active
    /// * `output_dir` - Directory to save the file in
    /// * `subtitle_langs` - Pre-selected subtitle language names (empty = config-based)
    /// * `audio_type_filter` - If set, filter lazily-resolved formats to this language
    #[allow(clippy::too_many_lines)]
    pub(super) async fn download_from_info_to_dir(
        &self,
        info: &rdlp_types::InfoDict,
        interactive: bool,
        output_dir: &std::path::Path,
        subtitle_langs: &[String],
        audio_type_filter: Option<&str>,
        batch_resolved_at: Option<Instant>,
    ) -> Result<Option<PathBuf>> {
        const MAX_EXTRACT_RETRIES: usize = 3;
        /// Exponential backoff delays: 5s, 15s, 30s — gives Cloudflare time
        /// to clear rate-limits between re-extraction attempts.
        const RETRY_DELAYS: [u64; MAX_EXTRACT_RETRIES] = [5, 15, 30];

        let mut output_path: Option<PathBuf> = None;
        let mut download_result: DownloadResult = None;
        let mut last_info_ref: Option<rdlp_types::InfoDict> = None;

        for attempt in 0..=MAX_EXTRACT_RETRIES {
            if attempt > 0 {
                #[allow(clippy::indexing_slicing)]
                // attempt ∈ 1..=MAX_EXTRACT_RETRIES; array len == MAX_EXTRACT_RETRIES
                let delay = Duration::from_secs(RETRY_DELAYS[attempt - 1]);
                warn!(
                    attempt = attempt + 1,
                    max = MAX_EXTRACT_RETRIES + 1,
                    delay_secs = delay.as_secs();
                    "Re-extracting fresh CDN URLs after backoff"
                );
                tokio::time::sleep(delay).await;
            }

            // Lazy resolve: always re-extract on retry, only if empty on first
            let resolved_info;
            // Re-extract if: retry attempt, no formats, or batch tokens expired
            let tokens_stale =
                attempt == 0 && batch_resolved_at.is_some_and(|t| t.elapsed() > TOKEN_MAX_AGE);
            if tokens_stale {
                debug!(
                    "Batch-resolved tokens stale (>{} min), forcing re-extraction",
                    TOKEN_MAX_AGE.as_secs() / 60
                );
            }
            let info_ref = if (attempt > 0 || info.formats.is_empty() || tokens_stale)
                && !info.webpage_url.is_empty()
            {
                debug!(title:? = info.title; "Lazily resolving episode formats");
                let mut resolved = match self.extract_lazy_formats(&info.webpage_url).await {
                    Ok(r) => r,
                    Err(e) if attempt < MAX_EXTRACT_RETRIES => {
                        warn!(
                            attempt = attempt + 1;
                            "Extraction failed: {e}, will retry"
                        );
                        continue;
                    }
                    Err(e) => return Err(e),
                };

                // Preserve all episode metadata from the original stub
                resolved.id.clone_from(&info.id);
                resolved.title.clone_from(&info.title);
                resolved.thumbnail.clone_from(&info.thumbnail);
                resolved.description.clone_from(&info.description);
                resolved.playlist.clone_from(&info.playlist);
                resolved.playlist_title.clone_from(&info.playlist_title);
                resolved.playlist_index = info.playlist_index;
                resolved.playlist_count = info.playlist_count;

                // Apply audio type filter to lazily-resolved formats
                if let Some(lang) = audio_type_filter {
                    resolved
                        .formats
                        .retain(|f| f.language.as_deref() == Some(lang));
                }

                resolved_info = resolved;
                &resolved_info
            } else {
                info
            };

            // Select format (non-interactive on retry to avoid re-prompting)
            let Some(plan) = self
                .select_format(info_ref, interactive && attempt == 0)
                .await?
            else {
                return Ok(None);
            };

            // Save info for post-download use (thumbnail, subtitles)
            last_info_ref = Some(info_ref.clone());

            match plan {
                DownloadPlan::Single(mut format) => {
                    // Add alternative CDN server URLs as fallbacks.
                    let mut alt_urls: Vec<String> = info_ref
                        .formats
                        .iter()
                        .filter(|f| f.url != format.url)
                        .map(|f| f.url.clone())
                        .collect();
                    if !alt_urls.is_empty() {
                        let primary_host = extract_cdn_host(&format.url);
                        alt_urls.sort_by_key(|url| u8::from(extract_cdn_host(url) != primary_host));
                        format
                            .fallback_urls
                            .get_or_insert_with(Vec::new)
                            .extend(alt_urls);
                    }

                    // Compute output path on first attempt only
                    if output_path.is_none() {
                        let file_ext = self.determine_file_extension(&format);
                        let sanitized_title = Self::sanitize_filename(&info_ref.title);
                        let filename = format!("{sanitized_title}.{file_ext}");
                        let path = output_dir.join(&filename);

                        self.cleanup_leftover_segments(output_dir, &sanitized_title)
                            .await;

                        debug!(path:? = path.display(); "Downloading to");
                        output_path = Some(path);
                    }

                    // output_path is always set: the is_none() guard above runs on first iteration.
                    #[allow(clippy::expect_used)]
                    // loop invariant: set by is_none guard on first pass
                    let path = output_path
                        .as_ref()
                        .expect("output_path set in is_none guard above");

                    // Download writes the deterministic .rdlp-part name; the clean
                    // name appears only after the download commits (#406).
                    let part = crate::orchestrator::naming::part_path(path);

                    // Detect resume point against the part name (recalculate on
                    // retry for partial HLS).
                    let resume_offset = self.detect_resume_point(&part, format.filesize).await?;

                    // Check if file is already complete: commit .rdlp-part -> clean.
                    if let Some(expected_size) = format.filesize
                        && resume_offset == expected_size
                    {
                        debug!("File already complete, skipping");
                        crate::orchestrator::naming::finalize_part(&part, path).await?;
                        return Ok(Some(path.clone()));
                    }

                    // Download with CDN fallback
                    match self
                        .download_with_cdn_fallback(&format, &part, resume_offset)
                        .await
                    {
                        Ok(Some(outcome)) => {
                            // Seam: move the finished .rdlp-part into the pipeline's
                            // own .rdlp-tmp-{uuid} namespace; the clean name is
                            // minted only after PP by the coordinator finalize (#406).
                            let seam = crate::orchestrator::naming::seam_path(path);
                            crate::orchestrator::naming::finalize_part(&part, &seam).await?;
                            download_result = Some((vec![seam], outcome.is_hls, None));
                            break;
                        }
                        Ok(None) => {
                            // #413: interrupt keeps the resumable partial; explicit
                            // cancel (default) deletes it.
                            if !self.should_keep_partial() {
                                crate::orchestrator::naming::discard_part(&part).await;
                            }
                            return Ok(None);
                        }
                        Err(e) if attempt < MAX_EXTRACT_RETRIES && is_reextractable_error(&e) => {
                            warn!("All CDN URLs failed: {e}");
                            warn!("Will re-extract fresh URLs after delay");
                        }
                        Err(e) => return Err(e),
                    }
                }
                DownloadPlan::Merge {
                    mut video,
                    mut audio,
                } => {
                    // Add CDN fallback URLs for both streams
                    let video_alts: Vec<String> = info_ref
                        .formats
                        .iter()
                        .filter(|f| f.url != video.url && f.has_video() && !f.has_audio())
                        .map(|f| f.url.clone())
                        .collect();
                    if !video_alts.is_empty() {
                        let primary_host = extract_cdn_host(&video.url);
                        let mut sorted = video_alts;
                        sorted.sort_by_key(|u| u8::from(extract_cdn_host(u) != primary_host));
                        video
                            .fallback_urls
                            .get_or_insert_with(Vec::new)
                            .extend(sorted);
                    }
                    let audio_alts: Vec<String> = info_ref
                        .formats
                        .iter()
                        .filter(|f| f.url != audio.url && f.has_audio() && !f.has_video())
                        .map(|f| f.url.clone())
                        .collect();
                    if !audio_alts.is_empty() {
                        let primary_host = extract_cdn_host(&audio.url);
                        let mut sorted = audio_alts;
                        sorted.sort_by_key(|u| u8::from(extract_cdn_host(u) != primary_host));
                        audio
                            .fallback_urls
                            .get_or_insert_with(Vec::new)
                            .extend(sorted);
                    }

                    // Compute output path on first attempt only
                    if output_path.is_none() {
                        let file_ext = self.determine_file_extension(&video);
                        let sanitized_title = Self::sanitize_filename(&info_ref.title);
                        let filename = format!("{sanitized_title}.{file_ext}");
                        let path = output_dir.join(&filename);

                        self.cleanup_leftover_segments(output_dir, &sanitized_title)
                            .await;

                        debug!(path:? = path.display(); "Downloading merge to");
                        output_path = Some(path);
                    }

                    // output_path is always set: the is_none() guard above runs on first iteration.
                    #[allow(clippy::expect_used)]
                    // loop invariant: set by is_none guard on first pass
                    let path = output_path
                        .as_ref()
                        .expect("output_path set in is_none guard above");

                    // Parallel download of video + audio
                    match self.download_merge_pair(&video, &audio, path).await {
                        Ok(Some(merge_outcome)) => {
                            download_result = Some((
                                vec![merge_outcome.video_path, merge_outcome.audio_path],
                                merge_outcome.is_hls,
                                Some(vec![video, audio]),
                            ));
                            break;
                        }
                        Ok(None) => return Ok(None),
                        Err(e) if attempt < MAX_EXTRACT_RETRIES && is_reextractable_error(&e) => {
                            warn!("Merge download failed: {e}");
                            warn!("Will re-extract fresh URLs after delay");
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }

        let (download_files, is_hls, merge_formats) = download_result.ok_or_else(|| {
            OrchestratorError::DownloadFailed(rdlp_core::RdlpError::Extraction {
                message: "All extraction retry attempts exhausted".to_string(),
                url: None,
            })
        })?;

        let output_path = output_path.ok_or_else(|| {
            OrchestratorError::DownloadFailed(rdlp_core::RdlpError::Extraction {
                message: "output path was not set during download".to_string(),
                url: None,
            })
        })?;
        let mut final_info_owned = last_info_ref.unwrap_or_else(|| info.clone());
        if let Some(fmts) = merge_formats {
            final_info_owned.requested_formats = Some(fmts);
        }
        let final_info = &final_info_owned;

        // Download thumbnail if needed (before post-processing so embed can find it)
        if self.config.postprocess.embed_thumbnail || self.config.postprocess.write_thumbnail {
            self.download_thumbnail(final_info, &output_path).await;
        }

        // Download subtitles: resolve per-episode URLs from language names.
        let episode_subs = if subtitle_langs.is_empty() {
            Vec::new()
        } else {
            self.resolve_subtitles_for_episode(final_info, subtitle_langs)
        };
        let downloaded_subs = self
            .download_subtitles(final_info, &output_path, &episode_subs)
            .await?;

        if !subtitle_langs.is_empty() && downloaded_subs.is_empty() {
            if self.config.strict_subs {
                return Err(OrchestratorError::DownloadFailed(
                    rdlp_core::RdlpError::Extraction {
                        message: format!(
                            "Strict subtitle mode: no subtitles downloaded for '{}' \
                             (expected [{}])",
                            final_info.title,
                            subtitle_langs.join(", ")
                        ),
                        url: None,
                    },
                ));
            }
            warn!(
                title:? = final_info.title,
                expected:% = subtitle_langs.join(", ");
                "Subtitles unavailable for this episode, continuing without"
            );
        }

        // Run post-processing if configured (or automatic for HLS).
        let final_files = self
            .run_postprocessing(final_info, download_files, is_hls, false)
            .await?;
        let survivor = final_files
            .into_iter()
            .next()
            .unwrap_or_else(|| output_path.clone());
        // Coordinator finalize (Option X / #412): single helper covers all paths.
        let final_path = crate::orchestrator::naming::finalize_survivor(survivor).await?;

        // HLS downloads produce .ts files that should be remuxed.
        if is_hls {
            let ext = final_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if ext.eq_ignore_ascii_case("ts") {
                return Err(OrchestratorError::DownloadFailed(
                    rdlp_core::RdlpError::Extraction {
                        message: format!(
                            "Post-processing failed for '{}': \
                             HLS container still .ts (FFmpeg remux did not complete)",
                            final_info.title,
                        ),
                        url: None,
                    },
                ));
            }
        }

        Ok(Some(final_path))
    }
}
