//! Playlist helper functions and batch-resolution logic

use super::super::{Orchestrator, Result};
use futures_util::StreamExt;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Whether `path` is still an un-remuxed MPEG-TS output, meaning an HLS
/// download's remux never completed.
///
/// One definition shared by the two callers that ask it — the post-download
/// check in `episode.rs` and the resume scan in `resume.rs` — which each
/// compared `eq_ignore_ascii_case("ts")` on their own before.
///
/// Recognises `ContainerFormat`'s aliases, so `.mpegts` counts too. These are
/// reject-guards, so widening the vocabulary makes them catch strictly more
/// incomplete output; it cannot be sidestepped by using the alias spelling.
pub(in crate::orchestrator) fn hls_remux_incomplete(path: &std::path::Path) -> bool {
    rdlp_types::ContainerFormat::from_path(path) == Some(rdlp_types::ContainerFormat::Ts)
}

impl Orchestrator {
    /// Handle the case where all videos are already downloaded.
    ///
    /// Performs subtitle retry if configured, then returns existing paths.
    pub(in crate::orchestrator) async fn handle_all_downloaded(
        &self,
        state_path: &std::path::Path,
        existing_files: &HashMap<String, PathBuf>,
        missing_subs: &HashMap<String, PathBuf>,
        saved_sub_langs: &[String],
        infos: &[rdlp_types::InfoDict],
        total: usize,
    ) -> Result<Option<Vec<PathBuf>>> {
        use super::super::session_state::SessionState;

        let need_sub_retry =
            self.config.retry_subs && !saved_sub_langs.is_empty() && !missing_subs.is_empty();
        if !need_sub_retry {
            if !missing_subs.is_empty() && !saved_sub_langs.is_empty() {
                // Report missing subs even when not retrying
                warn!("Missing subtitles for {} episode(s)", missing_subs.len());
                Self::log_missing_subs(missing_subs, infos, total);
                if !self.config.retry_subs {
                    info!("Hint: re-run with --retry-subs to attempt subtitle download");
                }
            }
            info!("All videos already downloaded");
            SessionState::delete(state_path).await;
            let paths: Vec<PathBuf> = existing_files.values().cloned().collect();
            return Ok(Some(paths));
        }
        // Fall through to subtitle retry pass below
        info!("All videos already downloaded — retrying missing subtitles");
        let sub_recovered = self
            .retry_missing_subtitles(missing_subs, infos, saved_sub_langs)
            .await;
        if sub_recovered > 0 {
            info!("Recovered subtitles for {sub_recovered} episode(s)");
        }
        let still_missing = missing_subs.len() - sub_recovered;
        if still_missing > 0 {
            warn!("Still missing subtitles for {still_missing} episode(s)");
        }
        SessionState::delete(state_path).await;
        let paths: Vec<PathBuf> = existing_files.values().cloned().collect();
        Ok(Some(paths))
    }

    /// Batch-resolve episode URLs with bounded parallelism.
    ///
    /// Returns the timestamp when resolution completed (for token freshness).
    pub(in crate::orchestrator) async fn batch_resolve_episodes(
        &self,
        infos: &mut [rdlp_types::InfoDict],
        existing_files: &HashMap<String, PathBuf>,
        selected_audio: Option<&str>,
    ) -> Instant {
        const RESOLVE_CONCURRENCY: usize = 4;

        // Collect work items: (index, url, title) for episodes needing resolution
        let to_resolve: Vec<(usize, String, String)> = infos
            .iter()
            .enumerate()
            .filter_map(|(i, ep)| {
                let sanitized = Self::sanitize_filename(&ep.title);
                if existing_files.contains_key(&sanitized) || ep.webpage_url.is_empty() {
                    None
                } else {
                    Some((i, ep.webpage_url.clone(), ep.title.clone()))
                }
            })
            .collect();

        let resolve_count = to_resolve.len();
        info!(count = resolve_count, concurrency = RESOLVE_CONCURRENCY; "Resolving episode URLs");

        // Resolve with buffer_unordered
        let futs = to_resolve.into_iter().map(|(i, url, title)| async move {
            debug!(position = i + 1, title:? = title; "Resolving episode");
            (i, self.extract_lazy_formats(&url).await)
        });

        let results: Vec<_> = futures_util::stream::iter(futs)
            .buffer_unordered(RESOLVE_CONCURRENCY)
            .collect()
            .await;

        #[allow(clippy::indexing_slicing)] // i comes from infos.enumerate(); always in-bounds
        for (i, result) in results {
            match result {
                Ok(resolved) => {
                    let mut formats = resolved.formats;

                    // Apply audio type filter
                    if let Some(lang) = selected_audio {
                        formats.retain(|f| f.language.as_deref() == Some(lang));
                    }

                    infos[i].formats = formats;

                    // Carry over subtitles from lazy resolution
                    if infos[i].subtitles.is_none() {
                        infos[i].subtitles = resolved.subtitles;
                    }
                }
                Err(e) => {
                    warn!(
                        title:? = infos[i].title;
                        "Failed to resolve: {e} — will retry at download time"
                    );
                    infos[i].formats.clear();
                }
            }
        }

        // Clear formats for already-downloaded episodes
        for ep in infos.iter_mut() {
            let sanitized = Self::sanitize_filename(&ep.title);
            if existing_files.contains_key(&sanitized) {
                ep.formats.clear();
            }
        }

        let batch_resolved_at = Instant::now();
        debug!("Batch resolution complete ({resolve_count} episodes)");
        batch_resolved_at
    }
}

/// Detect distinct audio/language types across all playlist episodes.
///
/// Scans format `language` fields and returns sorted unique values.
/// Used to offer SUB/DUB selection before batch download.
pub(super) fn detect_audio_types(infos: &[rdlp_types::InfoDict]) -> Vec<String> {
    let types: std::collections::HashSet<&str> = infos
        .iter()
        .flat_map(|info| &info.formats)
        .filter_map(|f| f.language.as_deref())
        .filter(|lang| !lang.is_empty())
        .collect();

    let mut result: Vec<String> = types.into_iter().map(String::from).collect();
    result.sort();
    result
}

/// Extract the hostname from a URL for CDN server stickiness.
///
/// Returns the host portion (e.g., "s2.netmagcdn.com") so fallback
/// URLs on the same CDN server can be prioritized.
pub(in crate::orchestrator) fn extract_cdn_host(url: &str) -> Option<&str> {
    let after_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    after_scheme.split('/').next()
}

/// Filter episode formats to only include the selected audio/language type.
///
/// Removes all formats whose `language` field does not match the given value.
pub(super) fn filter_formats_by_language(infos: &mut [rdlp_types::InfoDict], language: &str) {
    for info in infos.iter_mut() {
        info.formats
            .retain(|f| f.language.as_deref() == Some(language));
    }
}

#[cfg(test)]
mod hls_remux_incomplete_tests {
    use super::hls_remux_incomplete;
    use std::path::Path;

    #[test]
    fn mpeg_ts_outputs_are_incomplete() {
        for p in ["/tmp/Ep.ts", "/tmp/Ep.TS", "/tmp/Ep.mpegts"] {
            assert!(hls_remux_incomplete(Path::new(p)), "{p} is un-remuxed TS");
        }
    }

    /// The negative half: a completed remux, and the shapes that must not be
    /// mistaken for one.
    #[test]
    fn remuxed_and_unknown_outputs_are_not_incomplete() {
        for p in [
            "/tmp/Ep.mp4",
            "/tmp/Ep.mkv",
            "/tmp/Ep",
            "/tmp/Ep.rdlp-part",
            "/tmp/ts",
        ] {
            assert!(
                !hls_remux_incomplete(Path::new(p)),
                "{p} must not be flagged"
            );
        }
    }
}
