//! HLS format enrichment and multi-format size detection
//!
//! Public API for enriching `Format` objects with HLS metadata and expanding
//! master playlists into per-variant formats.

use super::detector::HlsSizeDetector;
use super::types::HlsStreamFlags;
use crate::base::common::BaseExtractor;
use log::debug;

/// Slugify a rendition tag (`LANGUAGE` / `GROUP-ID` / `NAME`) into a
/// format-id-safe token: lowercase ASCII alphanumerics with `-` for
/// any other character, collapsed and trimmed. Keeps audio-only format
/// ids predictable across re-extracts (e.g. `"English (5.1)"` → `english-5-1`).
fn slugify_tag(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = true; // suppress leading dashes
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

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
    format: &mut rdlp_types::Format,
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
    // Estimate file size from bitrate × duration (bytes = bps × seconds / 8)
    format.filesize_approx = match (hls_info.bandwidth, hls_info.total_duration) {
        (Some(bw), Some(dur)) => Some((bw as f64 * dur / 8.0) as u64),
        _ => None,
    };
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
/// duplication. HLS formats get variant expansion and metadata enrichment, while
/// non-HLS formats get file size detection via HEAD requests only when
/// `detect_sizes` is true. Passing `false` skips HEAD requests entirely,
/// deferring size detection to after format selection for faster startup.
///
/// # Arguments
/// * `formats` - Vector of formats to detect sizes for
/// * `ctx` - Extraction context with HTTP client and config
/// * `extractor_name` - Name of the extractor for logging (e.g., "PornHub", "RedTube")
///
/// # Returns
/// Tuple of (formats with sizes/segment counts populated, stream-level flags)
pub async fn detect_format_sizes(
    formats: Vec<rdlp_types::Format>,
    ctx: &rdlp_core::ExtractionContext,
    extractor_name: &str,
) -> (Vec<rdlp_types::Format>, HlsStreamFlags) {
    detect_format_sizes_inner(formats, ctx, extractor_name, true).await
}

/// Like [`detect_format_sizes`] but skips HEAD requests for non-HLS formats
/// when `detect_sizes` is false. HLS variant expansion always runs regardless.
pub async fn detect_format_sizes_lazy(
    formats: Vec<rdlp_types::Format>,
    ctx: &rdlp_core::ExtractionContext,
    extractor_name: &str,
) -> (Vec<rdlp_types::Format>, HlsStreamFlags) {
    detect_format_sizes_inner(formats, ctx, extractor_name, false).await
}

async fn detect_format_sizes_inner(
    formats: Vec<rdlp_types::Format>,
    ctx: &rdlp_core::ExtractionContext,
    extractor_name: &str,
    detect_sizes: bool,
) -> (Vec<rdlp_types::Format>, HlsStreamFlags) {
    use futures::future::join_all;
    use std::time::Duration;
    use tokio::time::timeout;

    let verbose = ctx.config.verbose;
    let mut hls_detector = HlsSizeDetector::new(ctx.http_client.clone(), verbose);

    // Propagate HTTP headers from formats (e.g., Referer) to the HLS detector.
    // Many CDNs (Megacloud/douvid.xyz) require a Referer to serve M3U8 content.
    if let Some(headers_map) = formats.iter().find_map(|f| f.http_headers.as_ref()) {
        let mut header_map = wreq::header::HeaderMap::new();
        for (key, value) in headers_map {
            if let (Ok(name), Ok(val)) = (
                wreq::header::HeaderName::from_bytes(key.as_bytes()),
                wreq::header::HeaderValue::from_str(value),
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
                        // Expand when the master produced multiple variants,
                        // OR when any audio-only rendition is present (even
                        // if there's only one video-only variant paired with
                        // it — the XHamster AV1 case).
                        Ok(Ok(v))
                            if v.len() > 1 || v.iter().any(|x| x.is_audio_only) =>
                        {
                            v
                        }
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

                    // Expand master playlist into one format per variant.
                    // Video/muxed entries use the resolution-based naming and
                    // inherit codec fallbacks from the parent `format`. Audio-
                    // only entries (derived from EXT-X-MEDIA TYPE=AUDIO
                    // rendition groups) are tagged with vcodec=`"none"` and
                    // named from the rendition (e.g. `hls-audio-en`), so the
                    // format selector can pair them with a video-only variant
                    // via `bv+ba`.
                    let mut expanded = Vec::with_capacity(variants.len());
                    for variant in &variants {
                        let height = variant.resolution.map(|(_, h)| h as u32);
                        let width = variant.resolution.map(|(w, _)| w as u32);
                        let format_id = if variant.is_audio_only {
                            // Prefer LANGUAGE, then GROUP-ID, then NAME; fall
                            // back to a bare `audio` suffix. Keeps ids stable
                            // across re-extracts.
                            let tag = variant
                                .language
                                .as_deref()
                                .or(variant.audio_group_id.as_deref())
                                .or(variant.rendition_name.as_deref())
                                .map(slugify_tag);
                            match tag {
                                Some(t) if !t.is_empty() => {
                                    format!("{}-audio-{t}", format.format_id)
                                }
                                _ => format!("{}-audio", format.format_id),
                            }
                        } else if let Some(h) = height {
                            format!("{}-{h}p", format.format_id)
                        } else {
                            format!("{}-{}k", format.format_id, variant.bandwidth / 1000)
                        };

                        let mut expanded_format = rdlp_types::Format::new(
                            &format_id,
                            &variant.media_playlist_url,
                            &format.ext,
                            format.protocol.clone(),
                        );
                        expanded_format.height = height;
                        expanded_format.width = width;
                        if variant.is_audio_only {
                            // Explicit "none" marker matches yt-dlp convention
                            // and is required for the selector's `ba` token to
                            // treat this row as audio-only (`has_video() == false`).
                            expanded_format.vcodec = Some("none".to_string());
                            expanded_format.acodec = variant
                                .audio_codec
                                .clone()
                                .or_else(|| Some("mp4a".to_string()));
                        } else {
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
                        }
                        expanded_format.fps = variant.frame_rate;
                        // EXT-X-MEDIA renditions carry no BANDWIDTH: leave
                        // tbr/filesize_approx unset rather than write misleading
                        // zeros.
                        expanded_format.tbr = if variant.bandwidth > 0 {
                            Some(variant.bandwidth as f64 / 1000.0)
                        } else {
                            None
                        };
                        expanded_format.http_headers = format.http_headers.clone();
                        // Surface the rendition language on the Format so the
                        // UI can label multi-language audio tracks. Fall back
                        // to the parent format's language for video variants.
                        expanded_format.language = variant
                            .language
                            .clone()
                            .or_else(|| format.language.clone());
                        // Propagate the HLS audio-rendition group:
                        //   - video-only rows carry the AUDIO= reference (the
                        //     group their paired audio lives in)
                        //   - audio-only rows carry their own GROUP-ID.
                        // UIs use matching values to visually pair rows when
                        // a user hand-picks without the Best preset.
                        expanded_format.audio_group_id = variant.audio_group_id.clone();
                        expanded_format.duration = variant.total_duration;
                        // Estimate size from bitrate × duration (bytes = bps × s / 8)
                        expanded_format.filesize_approx = match (variant.bandwidth, variant.total_duration) {
                            (bw, Some(dur)) if bw > 0 => Some((bw as f64 * dur / 8.0) as u64),
                            _ => None,
                        };
                        expanded_format.container = variant.segment_container.clone();
                        if variant.has_encryption {
                            expanded_format.has_drm = Some(true);
                        }
                        if variant.is_audio_only {
                            expanded_format.format_note = variant
                                .rendition_name
                                .clone()
                                .or_else(|| Some("audio".to_string()));
                        } else if let Some(h) = height {
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
                    // Non-HLS: HEAD request for file size (skipped when lazy)
                    let mut format = format;
                    if detect_sizes {
                        let result = timeout(
                            Duration::from_secs(5),
                            BaseExtractor::detect_file_size(&url, &http_client, None),
                        )
                        .await;

                        if let Ok(Some(size)) = result {
                            format.filesize = Some(size);
                        }
                    }

                    vec![(format, None, None)]
                }
            }
        })
        .collect();

    let results = join_all(detection_futures).await;

    // Flatten expanded formats, deduplicate HLS CDN mirrors, aggregate flags
    let mut formats: Vec<rdlp_types::Format> = Vec::new();
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

            // Deduplicate expanded HLS formats: keep format with largest estimated size
            // per (height, vcodec, acodec, language), collect other URLs as fallbacks.
            // Language is included so SUB/DUB tracks at the same resolution aren't merged.
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
                        // Keep the one with larger estimated size (more complete playlist)
                        let existing_size = existing.filesize_approx.unwrap_or(0);
                        let new_size = format.filesize_approx.unwrap_or(0);

                        if new_size > existing_size {
                            // New format has larger estimate - swap: make existing the fallback
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
