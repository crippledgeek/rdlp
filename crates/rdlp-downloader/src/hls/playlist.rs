//! HLS playlist parsing (m3u8).

use std::collections::HashSet;

use log::{debug, warn};
use rdlp_core::{RdlpError, Result};

use super::types::{InitSegmentInfo, PlaylistParseResult, SegmentInfo};
use crate::http::HttpDownloader;

/// Parse m3u8 playlist and extract segment URLs
///
/// Handles both media playlists (direct segments) and master playlists
/// (redirects to best variant). Uses recursive parsing for master playlists.
///
/// # Arguments
/// * `http_downloader` - HTTP downloader for fetching playlist
/// * `m3u8_url` - URL of the m3u8 playlist
///
/// # Returns
/// * `Ok(PlaylistParseResult)` - Segments and optional init segment URL
/// * `Err(_)` - Network error, parse error, or empty playlist
pub(crate) async fn parse_playlist(
    http_downloader: &HttpDownloader,
    m3u8_url: &str,
) -> Result<PlaylistParseResult> {
    // Fetch playlist text
    let playlist_text = http_downloader
        .client()
        .get(m3u8_url)
        .headers(http_downloader.headers())
        .send()
        .await
        .map_err(|e| RdlpError::Network { message: format!("Failed to fetch playlist: {e}"), url: Some(m3u8_url.to_string()) })?
        .text()
        .await
        .map_err(|e| RdlpError::Network { message: format!("Failed to read playlist: {e}"), url: Some(m3u8_url.to_string()) })?;

    // Parse with m3u8-rs
    let playlist = m3u8_rs::parse_playlist_res(playlist_text.as_bytes()).map_err(|e| {
        // Show the actual response content when parsing fails (e.g. CDN error pages)
        if !playlist_text.trim().starts_with("#EXTM3U") {
            let preview: String = playlist_text.chars().take(200).collect();
            RdlpError::Extraction {
                message: format!("Server returned invalid M3U8 (likely expired token or CDN error): {preview}"),
                url: Some(m3u8_url.to_string()),
            }
        } else {
            RdlpError::Extraction {
                message: format!("M3U8 parse error: {e:?}"),
                url: Some(m3u8_url.to_string()),
            }
        }
    })?;

    match playlist {
        m3u8_rs::Playlist::MediaPlaylist(media) => parse_media_playlist(media, m3u8_url),
        m3u8_rs::Playlist::MasterPlaylist(master) => {
            parse_master_playlist(http_downloader, master, m3u8_url).await
        }
    }
}

/// Parse a media playlist (direct segments)
fn parse_media_playlist(
    media: m3u8_rs::MediaPlaylist,
    m3u8_url: &str,
) -> Result<PlaylistParseResult> {
    // Warn about encryption (not yet supported)
    if media.segments.iter().any(|s| s.key.is_some()) {
        warn!("HLS stream uses encryption (AES-128/SAMPLE-AES) — decryption not yet supported");
    }
    // Warn about live streams
    if !media.end_list {
        warn!("HLS stream appears to be live (no EXT-X-ENDLIST) — may not download completely");
    }

    // Direct media playlist - extract segments with durations
    let base_url = url::Url::parse(m3u8_url)
        .map_err(|e| RdlpError::Extraction { message: format!("Invalid base URL: {e}"), url: Some(m3u8_url.to_string()) })?;

    // Build per-segment init info from EXT-X-MAP.
    // m3u8_rs sets `seg.map` on each segment the tag applies to,
    // so we just map it directly — handles multiple EXT-X-MAP tags.
    let segments: Vec<SegmentInfo> = media
        .segments
        .iter()
        .map(|seg| {
            let url = base_url
                .join(&seg.uri)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| seg.uri.clone());

            let init_segment = seg.map.as_ref().map(|map| {
                let init_url = base_url
                    .join(&map.uri)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|_| map.uri.clone());
                InitSegmentInfo {
                    url: init_url,
                    byte_range: map.byte_range.as_ref().map(|br| (br.length, br.offset)),
                }
            });

            SegmentInfo {
                url,
                duration: seg.duration as f64,
                init_segment,
            }
        })
        .collect();

    if segments.is_empty() {
        return Err(RdlpError::Extraction { message: "Playlist has no segments".into(), url: Some(m3u8_url.to_string()) });
    }

    // Log init segment info
    let unique_inits: HashSet<_> = segments
        .iter()
        .filter_map(|s| s.init_segment.as_ref())
        .collect();
    if !unique_inits.is_empty() {
        debug!(
            count = unique_inits.len();
            "fMP4 stream detected (EXT-X-MAP init segments)"
        );
    }

    // Security check: limit max segments
    const MAX_SEGMENTS: usize = 10_000;
    if segments.len() > MAX_SEGMENTS {
        return Err(RdlpError::Extraction {
            message: format!("Playlist has too many segments: {} (max: {MAX_SEGMENTS})", segments.len()),
            url: Some(m3u8_url.to_string()),
        });
    }

    // Check for potentially incomplete playlists (XHamster CDN bug: first segment > 1)
    // Pattern: seg-N-... where N should be 1 for first segment
    if let Some(first_seg) = segments.first()
        && let Some(first_seg_num) = extract_segment_number(&first_seg.url)
        && first_seg_num > 1
    {
        let missing = first_seg_num - 1;
        warn!(
            "Playlist may be incomplete: first segment is #{first_seg_num}, \
                     missing {missing} segment(s) from the beginning (~{} seconds). \
                     Consider trying a different format (AV1, MP4) or the fallback URL.",
            missing * 4 // Typical ~4s per segment
        );
    }

    Ok(PlaylistParseResult { segments })
}

/// Parse a master playlist (select best variant and recursively parse)
async fn parse_master_playlist(
    http_downloader: &HttpDownloader,
    master: m3u8_rs::MasterPlaylist,
    m3u8_url: &str,
) -> Result<PlaylistParseResult> {
    if master.variants.is_empty() {
        return Err(RdlpError::Extraction {
            message: "Master playlist has no variants".into(),
            url: Some(m3u8_url.to_string()),
        });
    }

    let variant = master
        .variants
        .iter()
        .filter(|v| !v.is_i_frame)
        .max_by_key(|v| v.bandwidth)
        .or_else(|| master.variants.iter().max_by_key(|v| v.bandwidth))
        .expect("master playlist has at least one variant");

    let base_url = url::Url::parse(m3u8_url)
        .map_err(|e| RdlpError::Extraction { message: format!("Invalid base URL: {e}"), url: Some(m3u8_url.to_string()) })?;

    let media_playlist_url = base_url
        .join(&variant.uri)
        .map_err(|e| RdlpError::Extraction { message: format!("Failed to join URL: {e}"), url: Some(m3u8_url.to_string()) })?
        .to_string();

    debug!(
        variant:? = variant.uri,
        bandwidth = variant.bandwidth;
        "Master playlist detected, selecting variant"
    );

    // Recursively parse media playlist (will detect EXT-X-MAP there)
    Box::pin(parse_playlist(http_downloader, &media_playlist_url)).await
}

/// Extract segment number from a URL matching patterns like:
/// - `seg-1-v1-a1.ts` or `seg-3-v1-a1.m4s` (XHamster)
/// - `segment1.ts` or `segment-1.ts`
/// - Other common segment numbering schemes
fn extract_segment_number(url: &str) -> Option<u32> {
    // Get the filename/path component
    let path = url.split('?').next().unwrap_or(url);
    let filename = path.rsplit('/').next().unwrap_or(path);

    // Pattern 1: seg-N-... (XHamster style)
    if let Some(num_part) = filename.strip_prefix("seg-")
        && let Some(end) = num_part.find('-')
    {
        return num_part[..end].parse().ok();
    }

    // Pattern 2: segmentN or segment-N or segment_N
    if let Some(rest) = filename.strip_prefix("segment") {
        let rest = rest.trim_start_matches('-').trim_start_matches('_');
        if let Some(end) = rest.find(|c: char| !c.is_ascii_digit()) {
            return rest[..end].parse().ok();
        }
        return rest
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok();
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_segment_number_xhamster() {
        // XHamster patterns
        assert_eq!(extract_segment_number("seg-1-v1-a1.ts"), Some(1));
        assert_eq!(extract_segment_number("seg-3-v1-a1.ts"), Some(3));
        assert_eq!(extract_segment_number("seg-502-v1-a1.m4s"), Some(502));
        assert_eq!(
            extract_segment_number("https://cdn.example.com/path/seg-1-v1-a1.ts?token=abc"),
            Some(1)
        );
        assert_eq!(
            extract_segment_number("https://cdn.example.com/path/seg-3-v1-a1.ts?token=abc"),
            Some(3)
        );
    }

    #[test]
    fn test_extract_segment_number_common() {
        // Common patterns
        assert_eq!(extract_segment_number("segment1.ts"), Some(1));
        assert_eq!(extract_segment_number("segment-1.ts"), Some(1));
        assert_eq!(extract_segment_number("segment_10.ts"), Some(10));
        assert_eq!(extract_segment_number("segment100.m4s"), Some(100));
    }

    #[test]
    fn test_extract_segment_number_unknown() {
        // Patterns that don't match
        assert_eq!(extract_segment_number("video.ts"), None);
        assert_eq!(extract_segment_number("chunk-1.ts"), None);
        assert_eq!(extract_segment_number("media-0001.ts"), None);
    }
}
