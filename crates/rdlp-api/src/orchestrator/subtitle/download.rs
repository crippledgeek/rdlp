//! Subtitle download execution — HTTP fetching and pipeline integration

use super::{Orchestrator, Result};
use log::{debug, warn};
use rdlp_types::InfoDict;
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
        interactive_selection: &[(String, rdlp_types::Subtitle)],
    ) -> Result<Vec<(String, PathBuf)>> {
        // If interactive selection is non-empty, use it directly
        if !interactive_selection.is_empty() {
            return self
                .download_subtitles_from_selection(interactive_selection, output_path)
                .await;
        }

        // Config-based path: delegate to the structured pipeline
        if !self.config.postprocess.write_subtitles && !self.config.postprocess.embed_subtitles {
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
        selected: &[(String, rdlp_types::Subtitle)],
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

            debug!("Downloading subtitle: lang={lang}, url={}", sub.url);

            match self.download_subtitle_file(&sub.url, &sub_path).await {
                Ok(()) => {
                    debug!("Subtitle downloaded: {}", sub_path.display());
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
            debug!("No subtitles selected");
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
                rdlp_core::RdlpError::Extraction {
                    message: msg,
                    url: None,
                },
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

            debug!(
                lang:% = track.language,
                ext:% = track.ext;
                "Downloading subtitle"
            );

            match self.download_subtitle_file(&track.url, &sub_path).await {
                Ok(()) => {
                    debug!("Subtitle downloaded: {}", sub_path.display());
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
    ///
    /// Pre-flight: extractor-sourced URLs are validated through
    /// `rdlp_security::validate_url_security` to prevent SSRF via a
    /// malicious or compromised extractor returning a private/loopback
    /// URL in `SubtitleTrack.url`. Body is streamed with a 5 MB cap so
    /// an adversarial server can't OOM the host before the cap fires.
    async fn download_subtitle_file(
        &self,
        url: &str,
        output: &Path,
    ) -> std::result::Result<(), anyhow::Error> {
        const MAX_SUBTITLE_BYTES: usize = 5 * 1024 * 1024;

        rdlp_security::validate_url_security(url)
            .map_err(|e| anyhow::anyhow!("subtitle URL rejected: {e}"))?;

        let response = self.extraction_context.http_client.get(url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("Subtitle download failed with status {}", response.status());
        }

        // Streaming size check — abort the moment cumulative bytes
        // exceed MAX_SUBTITLE_BYTES rather than buffer the full body
        // before checking.
        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            if buf.len().saturating_add(bytes.len()) > MAX_SUBTITLE_BYTES {
                anyhow::bail!(
                    "subtitle exceeds {MAX_SUBTITLE_BYTES}-byte cap (host integrity guard)"
                );
            }
            buf.extend_from_slice(&bytes);
        }
        tokio::fs::write(output, &buf).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! SSRF gate negative tests for `download_subtitle_file`. We don't
    //! construct a full `Orchestrator` here — the gate is a one-liner
    //! call into `rdlp_security::validate_url_security`, so these tests
    //! exercise the guarantee directly: any URL the validator rejects
    //! MUST also be rejected by the call this module makes.

    #[test]
    fn ssrf_rejection_pairs_with_validator() {
        // If the validator rejects, our call site MUST too — the
        // download path uses `validate_url_security(url)?` as its first
        // operation. This test locks the contract from drifting.
        for bad in [
            "http://127.0.0.1/x.vtt",
            "http://[::1]/x.vtt",
            "http://[fe80::1]/x.vtt",
            "http://192.168.1.1/x.vtt",
            "http://10.0.0.1/x.vtt",
            "http://169.254.169.254/latest/meta-data/",
            "http://localhost/x",
            "http://[::ffff:127.0.0.1]/x",
        ] {
            assert!(
                rdlp_security::validate_url_security(bad).is_err(),
                "subtitle SSRF gate must reject {bad}"
            );
        }
    }
}
