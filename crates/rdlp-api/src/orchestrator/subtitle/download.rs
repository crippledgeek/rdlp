//! Subtitle download execution — HTTP fetching and pipeline integration

use super::{Orchestrator, Result};
use log::{debug, info, warn};
use rdlp_core::InfoDict;
use std::path::{Path, PathBuf};

impl Orchestrator {
    /// Download subtitles for a video.
    ///
    /// Uses pre-selected subtitles from interactive menu if provided,
    /// otherwise falls back to config-based selection.
    ///
    /// # Arguments
    /// * `info` - Video metadata with subtitle URLs
    /// * `output_path` - Path to the downloaded video file
    /// * `interactive_selection` - Pre-selected subtitles from interactive menu
    ///
    /// # Returns
    /// Vec of (language, path) for downloaded subtitle files
    pub(in crate::orchestrator) async fn download_subtitles(
        &self,
        info: &InfoDict,
        output_path: &Path,
        interactive_selection: &[(String, rdlp_core::Subtitle)],
    ) -> Result<Vec<(String, PathBuf)>> {
        // If interactive selection is non-empty, use it directly
        if !interactive_selection.is_empty() {
            return self
                .download_subtitles_from_selection(interactive_selection, output_path)
                .await;
        }

        // Config-based path: delegate to the structured pipeline
        if !self.config.write_subtitles && !self.config.embed_subtitles {
            return Ok(Vec::new());
        }

        let (downloaded, _warnings) = self
            .download_subtitles_with_pipeline(info, output_path)
            .await?;
        Ok(downloaded)
    }

    /// Download pre-selected subtitles (from interactive menu).
    ///
    /// # Arguments
    /// * `selected` - List of (lang, Subtitle) pairs from interactive selection
    /// * `output_path` - Path to the video file (subtitle paths derived from it)
    ///
    /// # Returns
    /// Vec of (language, path) for downloaded subtitle files
    pub(in crate::orchestrator) async fn download_subtitles_from_selection(
        &self,
        selected: &[(String, rdlp_core::Subtitle)],
        output_path: &Path,
    ) -> Result<Vec<(String, PathBuf)>> {
        let mut downloaded = Vec::new();

        for (lang, sub) in selected {
            let sub_path = super::super::container_resolver::sidecar_path(
                output_path,
                &format!("{lang}.{}", sub.ext),
            );

            if sub_path.exists() {
                debug!(path:? = sub_path.display(); "Subtitle already exists, skipping");
                downloaded.push((lang.clone(), sub_path));
                continue;
            }

            info!("Downloading subtitle: lang={lang}, url={}", sub.url);

            match self.download_subtitle_file(&sub.url, &sub_path).await {
                Ok(()) => {
                    info!("Subtitle downloaded: {}", sub_path.display());
                    downloaded.push((lang.clone(), sub_path));
                }
                Err(e) => {
                    warn!("Failed to download subtitle for {lang}: {e}");
                }
            }
        }

        Ok(downloaded)
    }

    /// Download only subtitles (no video) for `--list-subs-only` mode.
    ///
    /// Extracts metadata, shows interactive subtitle selection, downloads
    /// selected subtitle files, and returns their paths.
    ///
    /// # Arguments
    /// * `info` - Pre-extracted video metadata
    ///
    /// # Returns
    /// - `Ok(Some(paths))` - Downloaded subtitle file paths
    /// - `Ok(None)` - User cancelled
    pub(in crate::orchestrator) async fn download_subtitles_standalone(
        &self,
        info: &InfoDict,
    ) -> Result<Option<Vec<PathBuf>>> {
        let Some(selected) = self.select_subtitles_interactive(info).await? else {
            return Ok(None);
        };

        if selected.is_empty() {
            info!("No subtitles selected");
            return Ok(Some(Vec::new()));
        }

        // Generate output path from video title using resolved container
        let sanitized = self.sanitize_filename(&info.title);
        let output_stub = super::super::container_resolver::output_stub(
            &self.config,
            &self.config.output_directory,
            &sanitized,
        );

        let downloaded = self
            .download_subtitles_from_selection(&selected, &output_stub)
            .await?;

        let paths: Vec<PathBuf> = downloaded.into_iter().map(|(_, p)| p).collect();
        Ok(Some(paths))
    }

    /// Download subtitles using the structured pipeline with status reporting.
    ///
    /// Normalizes InfoDict subtitles into [`SubtitleResult`], optionally
    /// validates URLs, applies policy (language filtering, strict mode),
    /// and downloads. Returns downloaded paths and any warnings.
    pub(in crate::orchestrator) async fn download_subtitles_with_pipeline(
        &self,
        info: &InfoDict,
        output_path: &Path,
    ) -> Result<(Vec<(String, PathBuf)>, Vec<String>)> {
        use super::super::subtitle_pipeline::{
            apply_subtitle_policy, normalize_subtitles, validate_subtitle_urls,
        };

        // Stage 3: Normalize
        let mut result = normalize_subtitles(info);

        // Stage 2: Validate (optional, only if verify_sub_urls is true)
        if self.config.verify_sub_urls && result.has_tracks() {
            result = validate_subtitle_urls(result, &self.extraction_context.http_client).await;
        }

        // Stage 4: Policy
        let outcome = apply_subtitle_policy(
            &result,
            &self.config.subtitle_langs,
            self.config.subtitle_format,
            self.config.write_auto_subtitles,
            self.config.strict_subs,
        );

        // Log warnings
        for warning in &outcome.warnings {
            warn!("{warning}");
        }

        // Hard error in strict mode
        if outcome.should_fail {
            let msg = outcome
                .error_message
                .unwrap_or_else(|| "Subtitle policy check failed".to_string());
            return Err(super::super::OrchestratorError::DownloadFailed(
                rdlp_core::RdlpError::Extraction(msg),
            ));
        }

        // Download selected tracks
        let mut downloaded = Vec::new();

        for track in &outcome.selected {
            let sub_path = super::super::container_resolver::sidecar_path(
                output_path,
                &format!("{}.{}", track.language, track.ext),
            );

            if sub_path.exists() {
                debug!(path:? = sub_path.display(); "Subtitle already exists, skipping");
                downloaded.push((track.language.clone(), sub_path));
                continue;
            }

            info!(
                lang:% = track.language,
                ext:% = track.ext;
                "Downloading subtitle"
            );

            match self.download_subtitle_file(&track.url, &sub_path).await {
                Ok(()) => {
                    info!("Subtitle downloaded: {}", sub_path.display());
                    downloaded.push((track.language.clone(), sub_path));
                }
                Err(e) => {
                    warn!(
                        lang:% = track.language;
                        "Failed to download subtitle: {e}"
                    );
                }
            }
        }

        let warnings = outcome.warnings;
        Ok((downloaded, warnings))
    }

    /// Download a single subtitle file via HTTP.
    async fn download_subtitle_file(
        &self,
        url: &str,
        output: &Path,
    ) -> std::result::Result<(), anyhow::Error> {
        let response = self.extraction_context.http_client.get(url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("Subtitle download failed with status {}", response.status());
        }

        let bytes = response.bytes().await?;
        tokio::fs::write(output, &bytes).await?;
        Ok(())
    }
}
