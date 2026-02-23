//! HLS format enrichment and multi-format size detection
//!
//! Public API for enriching `Format` objects with HLS metadata and expanding
//! master playlists into per-variant formats.

use super::detector::HlsSizeDetector;
use super::types::HlsStreamFlags;
use crate::base::common::BaseExtractor;
use log::debug;

/// Detect video or audio codec from a format ID string.
///
/// Checks for common codec names embedded in format IDs like "hls-av1-url"
/// or "hls-h264-fallback". Returns `None` if no codec is detected.
fn detect_codec_from_id(format_id: &str, is_video: bool) -> Option<String> {
    let id = format_id.to_lowercase();
    if is_video {
        if id.contains("av1") || id.contains("av01") {
            Some("av1".to_string())
        } else if id.contains("h264") || id.contains("avc") {
            Some("h264".to_string())
        } else if id.contains("h265") || id.contains("hevc") || id.contains("hvc") {
            Some("hevc".to_string())
        } else if id.contains("vp9") || id.contains("vp09") {
            Some("vp9".to_string())
        } else {
            None
        }
    } else if id.contains("aac") || id.contains("mp4a") {
        Some("aac".to_string())
    } else if id.contains("opus") {
        Some("opus".to_string())
    } else {
        None
    }
}

/// Enrich a single HLS format with metadata from `detect_hls_metadata()`.
///
/// Used as a fallback when the HLS URL is a media playlist (not a master)
/// or when variant expansion fails.
///
/// Returns `(Option<bool>, Option<bool>)` — `(is_live, has_encryption)`.
async fn enrich_single_hls_format(
    format: &mut rdlp_core::Format,
    hls_detector: &HlsSizeDetector,
    url: &str,
    extractor_name: &str,
    verbose: bool,
) -> (Option<bool>, Option<bool>) {
    use std::time::Duration;
    use tokio::time::timeout;

    let result = timeout(
        Duration::from_secs(10),
        hls_detector.detect_hls_metadata(url),
    )
    .await;

    let hls_info = match result {
        Ok(Ok(Some(info))) => info,
        _ => {
            if verbose {
                debug!(
                    extractor:? = extractor_name,
                    format:? = format.format_id;
                    "HLS metadata detection failed or timed out"
                );
            }
            return (None, None);
        }
    };

    // Log before moving fields out of hls_info
    if verbose {
        debug!(
            extractor:? = extractor_name,
            format:? = format.format_id,
            resolution:? = hls_info.resolution,
            segments = hls_info.segment_count;
            "HLS single format enriched"
        );
    }

    let is_live = hls_info.is_live;
    let has_encryption = hls_info.has_encryption;

    // Enrich format with metadata — move owned fields to avoid cloning.
    if let Some((w, h)) = hls_info.resolution {
        format.width = Some(w as u32);
        format.height = Some(h as u32);
        format.format_note = Some(format!("{h}p"));
    }
    if hls_info.video_codec.is_some() {
        format.vcodec = hls_info.video_codec;
    }
    if hls_info.audio_codec.is_some() {
        format.acodec = hls_info.audio_codec;
    }
    format.fps = hls_info.frame_rate;
    if let Some(bw) = hls_info.bandwidth {
        format.tbr = Some(bw as f64 / 1000.0);
    }
    format.duration = hls_info.total_duration;
    format.filesize_approx = Some(hls_info.segment_count as u64);
    format.container = hls_info.segment_container;
    if has_encryption {
        format.has_drm = Some(true);
    }
    if let Some(h) = format.height {
        format.quality = Some((h / 100) as i32);
    }

    (Some(is_live), Some(has_encryption))
}

/// Detect file sizes and segment counts for all formats in parallel
///
/// This is a shared utility function used by multiple extractors to avoid code
/// duplication. HLS formats get fast segment counting (no size fetching), while
/// other formats get file size detection via HEAD requests.
///
/// # Arguments
/// * `formats` - Vector of formats to detect sizes for
/// * `ctx` - Extraction context with HTTP client and config
/// * `extractor_name` - Name of the extractor for logging (e.g., "PornHub", "RedTube")
///
/// # Returns
/// Tuple of (formats with sizes/segment counts populated, stream-level flags)
pub async fn detect_format_sizes(
    formats: Vec<rdlp_core::Format>,
    ctx: &rdlp_core::ExtractionContext,
    extractor_name: &str,
) -> (Vec<rdlp_core::Format>, HlsStreamFlags) {
    use futures::future::join_all;
    use std::time::Duration;
    use tokio::time::timeout;

    let verbose = ctx.config.verbose;
    let mut hls_detector = HlsSizeDetector::new(ctx.http_client.clone(), verbose);

    // Propagate HTTP headers from formats (e.g., Referer) to the HLS detector.
    // Many CDNs (Megacloud/douvid.xyz) require a Referer to serve M3U8 content.
    if let Some(headers_map) = formats.iter().find_map(|f| f.http_headers.as_ref()) {
        let mut header_map = reqwest::header::HeaderMap::new();
        for (key, value) in headers_map {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                header_map.insert(name, val);
            }
        }
        if !header_map.is_empty() {
            hls_detector = hls_detector.with_default_headers(header_map);
        }
    }

    let http_client = ctx.http_client.clone();
    let extractor_name = extractor_name.to_string();

    let detection_futures: Vec<_> = formats
        .into_iter()
        .map(|format| {
            let hls_detector = hls_detector.clone();
            let http_client = http_client.clone();
            let extractor_name = extractor_name.clone();

            async move {
                let url = format.url.clone();
                let is_hls = format.ext == "hls" || url.contains(".m3u8") || url.contains("/hls/");

                if is_hls {
                    // Try to expand master playlist into per-variant formats
                    let result = timeout(
                        Duration::from_secs(10),
                        hls_detector.detect_hls_variants(&url),
                    )
                    .await;

                    let variants = match result {
                        Ok(Ok(v)) if v.len() > 1 => v,
                        _ => {
                            // Not a master playlist or detection failed — fall back to
                            // single-format enrichment via detect_hls_metadata
                            let mut format = format;
                            let (is_live, has_enc) = enrich_single_hls_format(
                                &mut format,
                                &hls_detector,
                                &url,
                                &extractor_name,
                                verbose,
                            )
                            .await;
                            return vec![(format, is_live, has_enc)];
                        }
                    };

                    // Expand master playlist into one format per variant
                    let mut expanded = Vec::with_capacity(variants.len());
                    for variant in &variants {
                        let height = variant.resolution.map(|(_, h)| h as u32);
                        let width = variant.resolution.map(|(w, _)| w as u32);
                        let format_id = if let Some(h) = height {
                            format!("{}-{h}p", format.format_id)
                        } else {
                            format!("{}-{}k", format.format_id, variant.bandwidth / 1000)
                        };

                        let mut expanded_format = rdlp_core::Format::new(
                            &format_id,
                            &variant.media_playlist_url,
                            &format.ext,
                            format.protocol.clone(),
                        );
                        expanded_format.height = height;
                        expanded_format.width = width;
                        expanded_format.vcodec = variant
                            .video_codec
                            .clone()
                            .or_else(|| format.vcodec.clone())
                            .or_else(|| detect_codec_from_id(&format.format_id, true));
                        expanded_format.acodec = variant
                            .audio_codec
                            .clone()
                            .or_else(|| format.acodec.clone())
                            .or_else(|| detect_codec_from_id(&format.format_id, false));
                        expanded_format.fps = variant.frame_rate;
                        expanded_format.tbr = Some(variant.bandwidth as f64 / 1000.0);
                        expanded_format.http_headers = format.http_headers.clone();
                        expanded_format.language = format.language.clone();
                        expanded_format.filesize_approx = Some(variant.segment_count as u64);
                        expanded_format.duration = variant.total_duration;
                        expanded_format.container = variant.segment_container.clone();
                        if variant.has_encryption {
                            expanded_format.has_drm = Some(true);
                        }
                        if let Some(h) = height {
                            expanded_format.format_note = Some(format!("{h}p"));
                            expanded_format.quality = Some((h / 100) as i32);
                        }

                        let is_live = Some(variant.is_live);
                        let has_enc = Some(variant.has_encryption);
                        expanded.push((expanded_format, is_live, has_enc));
                    }

                    if verbose {
                        debug!(
                            extractor:? = extractor_name,
                            parent:? = format.format_id,
                            variants = expanded.len();
                            "HLS master expanded into per-quality formats"
                        );
                    }

                    expanded
                } else {
                    // Non-HLS: HEAD request for file size
                    let mut format = format;
                    let result = timeout(
                        Duration::from_secs(5),
                        BaseExtractor::detect_file_size(&url, &http_client, None),
                    )
                    .await;

                    if let Ok(Some(size)) = result {
                        format.filesize = Some(size);
                    }

                    vec![(format, None, None)]
                }
            }
        })
        .collect();

    let results = join_all(detection_futures).await;

    // Flatten expanded formats, deduplicate HLS CDN mirrors, aggregate flags
    let mut formats: Vec<rdlp_core::Format> = Vec::new();
    let mut flags = HlsStreamFlags::default();
    // Key: (height, vcodec, acodec, language) — language prevents merging
    // different audio tracks (e.g. SUB/DUB) at the same resolution.
    type HlsDedup = (Option<u32>, Option<String>, Option<String>, Option<String>);
    let mut seen_hls: std::collections::HashSet<HlsDedup> = std::collections::HashSet::new();

    for format_group in results {
        for (format, is_live, has_encryption) in format_group {
            if is_live.unwrap_or(false) {
                flags.is_live = true;
            }
            if has_encryption.unwrap_or(false) {
                flags.has_any_drm = true;
            }

            // Deduplicate expanded HLS formats: keep format with most segments per
            // (height, vcodec, acodec, language), collect other URLs as fallbacks.
            // Language is included so SUB/DUB tracks at the same resolution aren't merged.
            // Note: HLS segment count is stored in filesize_approx during extraction.
            if format.is_hls() {
                let key = (
                    format.height,
                    format.vcodec.clone(),
                    format.acodec.clone(),
                    format.language.clone(),
                );
                if !seen_hls.insert(key) {
                    // Find existing format with same key
                    if let Some(existing) = formats.iter_mut().find(|f| {
                        f.is_hls()
                            && f.height == format.height
                            && f.vcodec == format.vcodec
                            && f.acodec == format.acodec
                            && f.language == format.language
                    }) {
                        // Compare segment counts (stored in filesize_approx for HLS)
                        // Keep the one with more segments (more complete playlist)
                        let existing_segments = existing.filesize_approx.unwrap_or(0);
                        let new_segments = format.filesize_approx.unwrap_or(0);

                        if new_segments > existing_segments {
                            // New format has more segments - swap: make existing the fallback
                            let old_url = std::mem::replace(&mut existing.url, format.url.clone());
                            existing.filesize_approx = format.filesize_approx;
                            existing.duration = format.duration;
                            existing.filesize = format.filesize;
                            existing.fallback_urls.get_or_insert_default().push(old_url);
                        } else {
                            // Existing has equal or more segments - keep it, add new as fallback
                            existing
                                .fallback_urls
                                .get_or_insert_default()
                                .push(format.url.clone());
                        }
                    }
                    continue;
                }
            }

            formats.push(format);
        }
    }

    (formats, flags)
}
