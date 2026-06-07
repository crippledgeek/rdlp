//! Playlist extraction for PornHub
//!
//! Handles playlist pagination and video extraction.
//!
//! # Performance
//!
//! Video extraction uses parallel processing with `buffer_unordered` to extract
//! multiple videos concurrently (default: 4). This significantly speeds up playlist
//! extraction compared to sequential processing.

use crate::base::common::MAX_PLAYLIST_SIZE;
use futures::stream::{self, StreamExt};
use log::{debug, info, warn};
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result, check_http_response};
use rdlp_types::InfoDict;
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;

/// Timeout for extracting a single video (30 seconds)
const VIDEO_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Rate limit delay between pages (500ms)
const PAGE_RATE_LIMIT_MS: u64 = 500;

/// Number of concurrent video extractions (balance speed vs rate limiting)
const CONCURRENT_EXTRACTIONS: usize = 4;

use super::PornHubExtractor;
use super::patterns::{AJAX_TOKEN_PATTERN, VIDEO_COUNT_PATTERN, VIDEO_LINK_PATTERN};
use super::utils::{extract_host, set_age_cookies};

/// Pagination metadata
struct PaginationInfo {
    playlist_id: String,
    video_count: Option<usize>,
    token: Option<String>,
}

/// Extract all videos from a playlist
pub async fn extract_playlist(
    extractor: &PornHubExtractor,
    url: &str,
    ctx: &ExtractionContext,
) -> Result<Vec<InfoDict>> {
    let playlist_id =
        super::patterns::extract_playlist_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!(
                "Could not extract playlist ID: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        })?;

    let host = extract_host(url);

    // Set age verification cookies
    set_age_cookies(&host, ctx).await?;

    debug!(playlist_id:?; "[PornHub] Extracting playlist");

    // Fetch first page
    let response = ctx
        .http_client
        .get(url)
        .send()
        .await
        .map_err(|e| RdlpError::Network {
            message: format!("Failed to fetch playlist: {e}"),
            url: Some(url.to_string().into()),
        })?;

    check_http_response(&response)?;

    let webpage = response.text().await.map_err(|e| RdlpError::Network {
        message: format!("Failed to read response: {e}"),
        url: Some(url.to_string().into()),
    })?;

    // Extract metadata
    let (playlist_title, pagination_info, mut all_video_urls) = {
        let html = Html::parse_document(&webpage);

        let title = extract_playlist_title(&html, &playlist_id);
        let pagination = extract_pagination_info(&webpage, &playlist_id);
        let videos = extract_video_urls(&webpage, &host);

        (title, pagination, videos)
    };

    info!(title:? = playlist_title; "[PornHub] Playlist");
    debug!(
        video_count:? = pagination_info.video_count,
        has_token = pagination_info.token.is_some();
        "[PornHub] Playlist info"
    );

    if all_video_urls.is_empty() {
        return Err(RdlpError::Extraction {
            message: format!(
                "No videos found in playlist: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        });
    }

    // Handle pagination
    if let Some(video_count) = pagination_info.video_count {
        let page_count = calculate_page_count(video_count);

        debug!(video_count, page_count; "[PornHub] Paginated playlist");

        // Fetch remaining pages
        for page_num in 2..=page_count {
            debug!(page = page_num; "[PornHub] Fetching page");

            match download_page(page_num, &pagination_info, &host, ctx).await {
                Ok(page_html) => {
                    let page_videos = extract_video_urls(&page_html, &host);
                    if page_videos.is_empty() {
                        break;
                    }
                    all_video_urls.extend(page_videos);
                }
                Err(e) => {
                    debug!(page = page_num; "Failed to fetch page: {e}");
                    break;
                }
            }

            // Rate limiting
            tokio::time::sleep(Duration::from_millis(PAGE_RATE_LIMIT_MS)).await;
        }
    }

    let total = all_video_urls.len();
    debug!(total; "[PornHub] Found videos in playlist");

    // Security check: limit playlist size to prevent memory exhaustion
    if total > MAX_PLAYLIST_SIZE {
        return Err(RdlpError::Extraction {
            message: format!("Playlist too large: {total} videos (max: {MAX_PLAYLIST_SIZE})"),
            url: Some(url.to_string().into()),
        });
    }

    // Extract videos in parallel using buffer_unordered for concurrent processing
    debug!(total, concurrent = CONCURRENT_EXTRACTIONS; "[PornHub] Extracting videos");

    // Progress counter for verbose logging
    let completed = Arc::new(AtomicUsize::new(0));

    // Create extraction futures for all videos
    let extraction_futures = all_video_urls.into_iter().enumerate().map(
        |(index, (video_url, video_title_hint))| {
            let position = index + 1;
            let playlist_title = playlist_title.clone();
            let playlist_id = playlist_id.clone();
            let completed = Arc::clone(&completed);

            async move {
                // Use timeout to prevent hanging on slow/unresponsive servers
                let result =
                    timeout(VIDEO_EXTRACTION_TIMEOUT, extractor.extract(&video_url, ctx)).await;

                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;

                match result {
                    Ok(Ok(mut info)) => {
                        info.playlist = Some(playlist_title);
                        info.playlist_id = Some(playlist_id);
                        info.playlist_title = Some(info.playlist.clone().unwrap_or_default());
                        info.playlist_index = Some(position);
                        info.playlist_count = Some(total);

                        debug!(done, total, title:? = video_title_hint; "[PornHub] Extracted video");

                        Some((position, info))
                    }
                    Ok(Err(e)) => {
                        // Bumped from debug → warn so users see silent-pruning.
                        warn!(
                            position, total, title:? = video_title_hint;
                            "[PornHub] Failed to extract playlist item: {e}"
                        );
                        None
                    }
                    Err(_) => {
                        warn!(position, total, title:? = video_title_hint; "[PornHub] Timed out extracting playlist item");
                        None
                    }
                }
            }
        },
    );

    // Process extractions concurrently with bounded parallelism
    let results: Vec<Option<(usize, InfoDict)>> = stream::iter(extraction_futures)
        .buffer_unordered(CONCURRENT_EXTRACTIONS)
        .collect()
        .await;

    // Collect successful extractions and sort by playlist position
    let mut extracted: Vec<(usize, InfoDict)> = results.into_iter().flatten().collect();

    // Sort by playlist position to maintain order
    extracted.sort_by_key(|(pos, _)| *pos);

    // Extract just the InfoDict, discarding the position
    let results: Vec<InfoDict> = extracted.into_iter().map(|(_, info)| info).collect();

    if results.is_empty() {
        return Err(RdlpError::Extraction {
            message: format!(
                "Failed to extract any videos from playlist: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        });
    }

    let extracted = results.len();
    info!(extracted, total; "[PornHub] Successfully extracted videos");
    if extracted < total {
        warn!(
            extracted,
            total;
            "[PornHub] {} of {} playlist items could not be extracted",
            total - extracted,
            total
        );
    }

    Ok(results)
}

/// Extract playlist title from HTML
fn extract_playlist_title(html: &Html, playlist_id: &str) -> String {
    // Try h1 element
    if let Ok(selector) = Selector::parse("h1")
        && let Some(element) = html.select(&selector).next()
    {
        let text: String = element.text().collect();
        let text = text.trim();
        if !text.is_empty() && !text.eq_ignore_ascii_case("pornhub") {
            return text.to_string();
        }
    }

    // Try og:title meta tag
    if let Ok(selector) = Selector::parse("meta[property='og:title']")
        && let Some(element) = html.select(&selector).next()
        && let Some(content) = element.value().attr("content")
        && !content.is_empty()
        && !content.eq_ignore_ascii_case("pornhub")
    {
        return content.to_string();
    }

    format!("Playlist {playlist_id}")
}

/// Extract video URLs from page HTML
fn extract_video_urls(webpage: &str, host: &str) -> Vec<(String, String)> {
    let mut videos = Vec::new();
    let mut seen = HashSet::new();

    for caps in VIDEO_LINK_PATTERN.captures_iter(webpage) {
        if let Some(video_id) = caps.get(2) {
            let video_url = format!(
                "https://{}/view_video.php?viewkey={}",
                host,
                video_id.as_str()
            );

            if !seen.insert(video_url.clone()) {
                continue;
            }

            let title = caps
                .get(3)
                .map(|m| m.as_str().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Video {}", video_id.as_str()));

            debug!(video_id:? = video_id.as_str(), title:?; "[PornHub] Found video");

            videos.push((video_url, title));
        }
    }

    videos
}

/// Extract pagination info from JavaScript
fn extract_pagination_info(webpage: &str, playlist_id: &str) -> PaginationInfo {
    let video_count = VIDEO_COUNT_PATTERN
        .captures(webpage)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok());

    let token = AJAX_TOKEN_PATTERN
        .captures(webpage)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    PaginationInfo {
        playlist_id: playlist_id.to_string(),
        video_count,
        token,
    }
}

/// Calculate page count from video count
///
/// Page 1: 36 videos, Pages 2+: 40 videos each
fn calculate_page_count(video_count: usize) -> usize {
    if video_count <= 36 {
        1
    } else {
        (video_count - 36).div_ceil(40) + 1
    }
}

/// Download playlist page via AJAX
async fn download_page(
    page_num: usize,
    pagination: &PaginationInfo,
    host: &str,
    ctx: &ExtractionContext,
) -> Result<String> {
    let url = format!("https://{host}/playlist/viewChunked");

    let response = ctx
        .http_client
        .post(&url)
        .form(&[
            ("id", pagination.playlist_id.as_str()),
            ("page", &page_num.to_string()),
            ("token", pagination.token.as_deref().unwrap_or("")),
        ])
        .send()
        .await
        .map_err(|e| RdlpError::Network {
            message: format!("Failed to fetch page {page_num}: {e}"),
            url: Some(url.clone().into()),
        })?;

    check_http_response(&response)?;

    response.text().await.map_err(|e| RdlpError::Network {
        message: format!("Failed to read page {page_num}: {e}"),
        url: Some(url.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_page_count() {
        assert_eq!(calculate_page_count(10), 1);
        assert_eq!(calculate_page_count(36), 1);
        assert_eq!(calculate_page_count(37), 2);
        assert_eq!(calculate_page_count(76), 2);
        assert_eq!(calculate_page_count(77), 3);
    }
}
