//! User/creator playlist extraction for xHamster
//!
//! Handles pagination and parallel video extraction from user and creator pages.

use super::XHamsterExtractor;
use super::patterns;
use crate::base::common::MAX_PLAYLIST_SIZE;
use futures::stream::{self, StreamExt};
use lazy_regex::{Lazy, Regex, lazy_regex};
use log::{debug, info, warn};
use rdlp_core::{
    ExponentialBuilder, ExtractionContext, RdlpError, Result, Retryable, check_http_response,
};
use rdlp_types::InfoDict;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;

use super::{CONCURRENT_EXTRACTIONS, PAGE_RATE_LIMIT_MS, VIDEO_EXTRACTION_TIMEOUT};

impl XHamsterExtractor {
    /// Extract all videos from a user/creator page with pagination.
    pub(super) async fn extract_user_playlist(
        &self,
        url: &str,
        ctx: &ExtractionContext,
    ) -> Result<Vec<InfoDict>> {
        let (user_id, _is_user) =
            patterns::extract_user_info(url).ok_or_else(|| RdlpError::Extraction {
                message: format!("Could not extract user ID: {url}"),
                url: Some(url.to_string()),
            })?;

        info!(user_id:?; "[XHamster] Extracting user playlist");

        let mut all_video_urls: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let mut page = 1;

        loop {
            let page_url = if page == 1 {
                url.to_string()
            } else {
                format!("{url}/{page}")
            };

            debug!(page, url:? = page_url; "[XHamster] Fetching user page");

            let response = (|| async { ctx.http_client.get(&page_url).send().await })
                .retry(
                    ExponentialBuilder::default()
                        .with_max_times(2)
                        .with_min_delay(Duration::from_millis(500)),
                )
                .when(|e| e.is_timeout() || e.is_connect())
                .await
                .map_err(|e| RdlpError::Network {
                    message: format!("Failed to fetch user page {page}: {e}"),
                    url: Some(page_url.clone()),
                })?;

            check_http_response(&response)?;

            let webpage = response.text().await.map_err(|e| RdlpError::Network {
                message: format!("Failed to read user page {page}: {e}"),
                url: Some(page_url.clone()),
            })?;

            // Extract video URLs from the page
            let page_urls = extract_user_video_urls(&webpage);
            if page_urls.is_empty() {
                break;
            }

            let mut found_new = false;
            for video_url in page_urls {
                if seen.insert(video_url.clone()) {
                    all_video_urls.push(video_url);
                    found_new = true;
                }
            }

            if !found_new {
                break;
            }

            // Check for next page link
            if !webpage.contains("data-page=\"next\"") {
                break;
            }

            page += 1;

            // Rate limiting
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }

        let total = all_video_urls.len();
        debug!(total; "[XHamster] Found videos in user playlist");

        if total == 0 {
            return Err(RdlpError::Extraction {
                message: format!("No videos found on user page: {url}"),
                url: Some(url.to_string()),
            });
        }

        if total > MAX_PLAYLIST_SIZE {
            return Err(RdlpError::Extraction {
                message: format!("Playlist too large: {total} videos (max: {MAX_PLAYLIST_SIZE})"),
                url: Some(url.to_string()),
            });
        }

        // Extract videos in parallel
        debug!(total, concurrent = CONCURRENT_EXTRACTIONS; "[XHamster] Extracting videos");

        let completed = Arc::new(AtomicUsize::new(0));

        let extraction_futures =
            all_video_urls
                .into_iter()
                .enumerate()
                .map(|(index, video_url)| {
                    let position = index + 1;
                    let user_id = user_id.clone();
                    let completed = Arc::clone(&completed);

                    async move {
                        let result = timeout(
                            VIDEO_EXTRACTION_TIMEOUT,
                            self.extract_video(&video_url, ctx),
                        )
                        .await;

                        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;

                        match result {
                            Ok(Ok(mut info)) => {
                                info.playlist = Some(user_id);
                                info.playlist_index = Some(position);
                                info.playlist_count = Some(total);

                                debug!(done, total; "[XHamster] Extracted video");
                                Some((position, info))
                            }
                            Ok(Err(e)) => {
                                // Bumped from debug → warn so playlist users see
                                // silent-pruning of items they expected to get.
                                warn!(position, total; "[XHamster] Failed to extract playlist item: {e}");
                                None
                            }
                            Err(_) => {
                                warn!(position, total; "[XHamster] Timed out extracting playlist item");
                                None
                            }
                        }
                    }
                });

        let results: Vec<Option<(usize, InfoDict)>> = stream::iter(extraction_futures)
            .buffer_unordered(CONCURRENT_EXTRACTIONS)
            .collect()
            .await;

        let mut extracted: Vec<(usize, InfoDict)> = results.into_iter().flatten().collect();
        extracted.sort_by_key(|(pos, _)| *pos);

        let results: Vec<InfoDict> = extracted.into_iter().map(|(_, info)| info).collect();

        if results.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!("Failed to extract any videos from user page: {url}"),
                url: Some(url.to_string()),
            });
        }

        let extracted = results.len();
        info!(extracted, total; "[XHamster] Successfully extracted videos");
        if extracted < total {
            // Loud aggregate so the user sees the prune count even when
            // they don't have RUST_LOG=warn on per-item lines.
            warn!(
                extracted,
                total;
                "[XHamster] {} of {} playlist items could not be extracted",
                total - extracted,
                total
            );
        }

        Ok(results)
    }
}

/// Extract video URLs from a user/creator page HTML.
///
/// Looks for `a.video-thumb__image-container` elements with href attributes.
pub(super) fn extract_user_video_urls(webpage: &str) -> Vec<String> {
    static VIDEO_THUMB_HREF: Lazy<Regex> = lazy_regex!(
        r#"<a[^>]+class=[\"'][^\"']*\bvideo-thumb__image-container[^>]+href=[\"']([^\"']+)[\"']"#
    );

    static VIDEO_THUMB_HREF_ALT: Lazy<Regex> = lazy_regex!(
        r#"<a[^>]+href=[\"']([^\"']+)[\"'][^>]+class=[\"'][^\"']*\bvideo-thumb__image-container"#
    );

    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    for pattern in [&*VIDEO_THUMB_HREF, &*VIDEO_THUMB_HREF_ALT] {
        for caps in pattern.captures_iter(webpage) {
            if let Some(href) = caps.get(1) {
                let url = href.as_str().to_string();
                if patterns::XHAMSTER_VIDEO_PATTERN.is_match(&url) && seen.insert(url.clone()) {
                    urls.push(url);
                }
            }
        }
    }

    urls
}
