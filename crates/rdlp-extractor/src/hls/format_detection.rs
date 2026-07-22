//! HLS format enrichment and multi-format size detection
//!
//! Public API for enriching `Format` objects with HLS metadata and expanding
//! master playlists into per-variant formats.

use super::detector::HlsSizeDetector;
use super::types::HlsStreamFlags;
use crate::base::common::BaseExtractor;
use log::debug;
use rdlp_types::Codec;

/// Default cap on the single-HEAD probe for non-HLS file size detection
/// when `Config::hls_head_probe_timeout` is unset. Matches the legacy
/// hard-coded value before #277.
const DEFAULT_HLS_HEAD_PROBE_TIMEOUT_SECS: u64 = 5;

/// Resolve the wall-clock cap for the non-HLS HEAD-probe from `Config`,
/// falling back to `DEFAULT_HLS_HEAD_PROBE_TIMEOUT_SECS` when unset.
pub(crate) fn resolve_hls_head_probe_timeout(config: &rdlp_types::Config) -> std::time::Duration {
    std::time::Duration::from_secs(
        config
            .hls_head_probe_timeout
            .unwrap_or(DEFAULT_HLS_HEAD_PROBE_TIMEOUT_SECS),
    )
}

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
    let result = hls_detector.detect_hls_metadata(url).await;

    let hls_info = match result {
        Ok(Some(info)) => info,
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
    if let Some(vc) = hls_info.video_codec {
        format.vcodec = Codec::from(vc.as_str());
    }
    if let Some(ac) = hls_info.audio_codec {
        format.acodec = Codec::from(ac.as_str());
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

/// Captured environment for a single per-format detection future.
///
/// Bundles all closure-captured values to keep `build_format_detection_future`
/// under the `too_many_arguments` threshold and to make the dependency set
/// explicit at the call site.
struct FormatDetectionCtx {
    hls_detector: HlsSizeDetector,
    http_client: std::sync::Arc<wreq::Client>,
    extractor_name: String,
    detect_sizes: bool,
    head_timeout: std::time::Duration,
    verbose: bool,
}

/// Write per-variant HLS labels from `variant` onto `format`, inheriting codec
/// and language fallbacks from `parent_format`.
///
/// Sets `format_id` (resolution-suffixed `…-720p` for video, `…-audio-{tag}`
/// for renditions), `height`/`width`, `vcodec`/`acodec`, `fps`, `tbr`,
/// `http_headers`, `language`, `audio_group_id`, `duration`, `filesize_approx`,
/// `container`, `has_drm`, `format_note`, and `quality`.
///
/// Does NOT touch `url` or `fragments` — the caller owns those. This is the
/// single labeling source shared by [`expand_hls_variant`] (the detect/size
/// path) and `expand::expand_hls_url` (the pre-resolved-fragments path), so the
/// two HLS expansion entry points produce identically-labeled rows.
pub(crate) fn apply_variant_labels(
    format: &mut rdlp_types::Format,
    variant: &super::types::HlsVariantInfo,
    parent_format: &rdlp_types::Format,
) {
    let height = variant.resolution.map(|(_, h)| h as u32);
    let width = variant.resolution.map(|(w, _)| w as u32);

    format.format_id = if variant.is_audio_only {
        // Prefer LANGUAGE, then GROUP-ID, then NAME; fall back to a bare
        // `audio` suffix. Keeps ids stable across re-extracts.
        let tag = variant
            .language
            .as_deref()
            .or(variant.audio_group_id.as_deref())
            .or(variant.rendition_name.as_deref())
            .map(slugify_tag);
        match tag {
            Some(t) if !t.is_empty() => format!("{}-audio-{t}", parent_format.format_id),
            _ => format!("{}-audio", parent_format.format_id),
        }
    } else if let Some(h) = height {
        format!("{}-{h}p", parent_format.format_id)
    } else {
        format!("{}-{}k", parent_format.format_id, variant.bandwidth / 1000)
    };

    format.height = height;
    format.width = width;

    if variant.is_audio_only {
        // Audio-only HLS rendition: video stream Absent;
        // ba selector treats as audio-only (has_video() == false).
        format.vcodec = Codec::Absent;
        format.acodec = Codec::from(
            variant
                .audio_codec
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str)
                .map(str::to_owned)
                .or_else(|| Some("mp4a".to_string())),
        );
    } else {
        format.vcodec = Codec::from(
            variant
                .video_codec
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str)
                .map(str::to_owned)
                .or_else(|| parent_format.vcodec.as_str().map(str::to_owned))
                .or_else(|| detect_codec_from_id(&parent_format.format_id, true)),
        );
        format.acodec = Codec::from(
            variant
                .audio_codec
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str)
                .map(str::to_owned)
                .or_else(|| parent_format.acodec.as_str().map(str::to_owned))
                .or_else(|| detect_codec_from_id(&parent_format.format_id, false)),
        );
    }

    format.fps = variant.frame_rate;
    // EXT-X-MEDIA renditions carry no BANDWIDTH: leave
    // tbr/filesize_approx unset rather than write misleading zeros.
    format.tbr = if variant.bandwidth > 0 {
        Some(variant.bandwidth as f64 / 1000.0)
    } else {
        None
    };
    format.http_headers = parent_format.http_headers.clone();
    // Surface the rendition language on the Format so the UI can label
    // multi-language audio tracks. Fall back to the parent format's language
    // for video variants.
    format.language = variant
        .language
        .clone()
        .or_else(|| parent_format.language.clone());
    // Propagate the HLS audio-rendition group:
    //   - video-only rows carry the AUDIO= reference (the group their paired
    //     audio lives in)
    //   - audio-only rows carry their own GROUP-ID.
    // UIs use matching values to visually pair rows when a user hand-picks
    // without the Best preset.
    format.audio_group_id = variant.audio_group_id.clone();
    // Only overwrite duration when the variant carries one (the pure
    // master-parse path leaves it None and the caller may have computed
    // duration from the fragment list).
    if let Some(dur) = variant.total_duration {
        format.duration = Some(dur);
    }
    // Estimate size from bitrate × duration (bytes = bps × s / 8). Falls back
    // to any duration already on the format (e.g. summed from fragments).
    let dur_for_size = variant.total_duration.or(format.duration);
    if let Some(dur) = dur_for_size
        && variant.bandwidth > 0
    {
        format.filesize_approx = Some((variant.bandwidth as f64 * dur / 8.0) as u64);
    }
    format.container = variant.segment_container.clone();
    if variant.has_encryption {
        format.has_drm = Some(true);
    }
    if variant.is_audio_only {
        format.format_note = variant
            .rendition_name
            .clone()
            .or_else(|| Some("audio".to_string()));
    } else if let Some(h) = height {
        format.format_note = Some(format!("{h}p"));
        format.quality = Some((h / 100) as i32);
    }
}

/// Expand a single HLS `variant` into a `Format`, inheriting metadata from
/// `parent_format` (ext, protocol, headers, codec fallbacks, language).
///
/// Returns `(expanded_format, is_live, has_encryption)`.
fn expand_hls_variant(
    variant: &super::types::HlsVariantInfo,
    parent_format: &rdlp_types::Format,
) -> DetectionEntry {
    let mut expanded_format = rdlp_types::Format::new(
        &parent_format.format_id,
        &variant.media_playlist_url,
        &parent_format.ext,
        parent_format.protocol.clone(),
    );
    apply_variant_labels(&mut expanded_format, variant, parent_format);

    (
        expanded_format,
        Some(variant.is_live),
        Some(variant.has_encryption),
    )
}

/// Build the async future that probes a single `format`.
///
/// HLS formats are expanded into per-variant entries (or enriched as a single
/// format when the URL is a media playlist). Non-HLS formats receive a HEAD
/// probe for file size when `ctx.detect_sizes` is `true`.
///
/// Returns a `Vec` so that one input format can expand into many (HLS master
/// with multiple variants).
async fn build_format_detection_future(
    format: rdlp_types::Format,
    ctx: FormatDetectionCtx,
) -> Vec<DetectionEntry> {
    use tokio::time::timeout;

    let url = format.url.clone();
    let is_hls = format.ext == "hls"
        || url::Url::parse(&url)
            .map(|u| {
                matches!(
                    crate::base::common::protocol_for_url(&u),
                    rdlp_types::DownloadProtocol::M3u8,
                )
            })
            .unwrap_or(false);

    if is_hls {
        // Already pre-resolved by `expand_hls_in_place`: the row is complete
        // (per-variant `url`, labels, and `fragments`). Re-expanding here would
        // re-fetch the playlist and produce fragment-less rows that clobber the
        // pre-resolved ones — the xhamster `hls-h264-url-2160p` bug. Pass
        // through untouched (it already carries duration/size from expansion).
        if format.fragments.is_some() {
            return vec![(format, None, None)];
        }

        // Try to expand master playlist into per-variant formats
        let variants_res = ctx.hls_detector.detect_hls_variants(&url).await;

        let variants = match variants_res {
            // Expand when the master produced multiple variants,
            // OR when any audio-only rendition is present (even
            // if there's only one video-only variant paired with
            // it — the XHamster AV1 case).
            Ok(v) if v.len() > 1 || v.iter().any(|x| x.is_audio_only) => v,
            _ => {
                // Not a master playlist or detection failed — fall back to
                // single-format enrichment via detect_hls_metadata
                let mut format = format;
                let (is_live, has_enc) = enrich_single_hls_format(
                    &mut format,
                    &ctx.hls_detector,
                    &url,
                    &ctx.extractor_name,
                    ctx.verbose,
                )
                .await;
                return vec![(format, is_live, has_enc)];
            }
        };

        // Expand master playlist into one format per variant.
        // Video/muxed entries use the resolution-based naming and inherit
        // codec fallbacks from the parent `format`. Audio-only entries
        // (derived from EXT-X-MEDIA TYPE=AUDIO rendition groups) are tagged
        // with vcodec=`"none"` and named from the rendition
        // (e.g. `hls-audio-en`), so the format selector can pair them with a
        // video-only variant via `bv+ba`.
        let mut expanded = Vec::with_capacity(variants.len());
        for variant in &variants {
            expanded.push(expand_hls_variant(variant, &format));
        }

        if ctx.verbose {
            debug!(
                extractor:? = ctx.extractor_name,
                parent:? = format.format_id,
                variants = expanded.len();
                "HLS master expanded into per-quality formats"
            );
        }

        expanded
    } else {
        // Non-HLS: HEAD request for file size (skipped when lazy)
        let mut format = format;
        if ctx.detect_sizes {
            let result = timeout(
                ctx.head_timeout,
                BaseExtractor::detect_file_size(&url, &ctx.http_client, None, ctx.head_timeout),
            )
            .await;

            if let Ok(Some(size)) = result {
                format.filesize = Some(size);
            }
        }

        vec![(format, None, None)]
    }
}

/// Single per-format detection result: enriched format plus optional stream flags.
///
/// `(format, is_live, has_encryption)` — flags are `None` for non-HLS formats.
type DetectionEntry = (rdlp_types::Format, Option<bool>, Option<bool>);

/// Flatten per-format detection results, deduplicate HLS CDN mirrors, and
/// aggregate stream-level flags.
///
/// Each inner `Vec` is the result of one `build_format_detection_future` call.
/// HLS formats with identical `(height, vcodec, acodec, language)` keys are
/// merged: the entry with the largest estimated size is kept as the primary
/// URL; others are appended to `fallback_urls`.
fn aggregate_results(
    results: Vec<Vec<DetectionEntry>>,
) -> (Vec<rdlp_types::Format>, HlsStreamFlags) {
    let mut formats: Vec<rdlp_types::Format> = Vec::new();
    let mut flags = HlsStreamFlags::default();
    // Key: (height, vcodec, acodec, language) — language prevents merging
    // different audio tracks (e.g. SUB/DUB) at the same resolution.
    type HlsDedup = (Option<u32>, Codec, Codec, Option<String>);
    let mut seen_hls: std::collections::HashSet<HlsDedup> = std::collections::HashSet::new();

    for format_group in results {
        for (format, is_live, has_encryption) in format_group {
            if is_live.unwrap_or(false) {
                flags.is_live = true;
            }
            if has_encryption.unwrap_or(false) {
                flags.has_any_drm = true;
            }

            // Deduplicate expanded HLS formats: keep format with largest estimated
            // size per (height, vcodec, acodec, language), collect other URLs as
            // fallbacks. Language is included so SUB/DUB tracks at the same
            // resolution aren't merged.
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
                            // New format has larger estimate — swap: make existing the fallback
                            let old_url = std::mem::replace(&mut existing.url, format.url.clone());
                            existing.filesize_approx = format.filesize_approx;
                            existing.duration = format.duration;
                            existing.filesize = format.filesize;
                            existing.fallback_urls.get_or_insert_default().push(old_url);
                        } else {
                            // Existing has equal or more segments — keep it, add new as fallback
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

async fn detect_format_sizes_inner(
    formats: Vec<rdlp_types::Format>,
    ctx: &rdlp_core::ExtractionContext,
    extractor_name: &str,
    detect_sizes: bool,
) -> (Vec<rdlp_types::Format>, HlsStreamFlags) {
    use futures::future::join_all;

    let verbose = ctx.config.verbose;
    let head_timeout = resolve_hls_head_probe_timeout(&ctx.config);
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
            let detection_ctx = FormatDetectionCtx {
                hls_detector: hls_detector.clone(),
                http_client: http_client.clone(),
                extractor_name: extractor_name.clone(),
                detect_sizes,
                head_timeout,
                verbose,
            };
            build_format_detection_future(format, detection_ctx)
        })
        .collect();

    let results = join_all(detection_futures).await;
    aggregate_results(results)
}

#[cfg(test)]
mod is_hls_tests {
    use crate::base::common::protocol_for_url;
    use rdlp_types::DownloadProtocol;
    use url::Url;

    /// Mirror of the post-migration `is_hls` decision used inside the
    /// detection futures combinator at line ~221. Kept as a free function
    /// so the test surface matches the production predicate exactly.
    fn is_hls_decision(ext: &str, url: &str) -> bool {
        ext == "hls"
            || Url::parse(url)
                .map(|u| matches!(protocol_for_url(&u), DownloadProtocol::M3u8))
                .unwrap_or(false)
    }

    fn url_is_hls(url: &str) -> bool {
        is_hls_decision("mp4", url)
    }

    #[test]
    fn rejects_mp4_url_with_m3u8_in_query() {
        // Regression: issue #268. The pre-migration check
        // `url.contains(".m3u8")` returned true here.
        assert!(!url_is_hls("https://host/clip.mp4?ref=foo.m3u8"));
    }

    #[test]
    fn rejects_mp4_url_with_hls_substring_in_path() {
        // Regression: pre-migration check `url.contains("/hls/")` returned
        // true. The CDN-style `/hls/` substring is no longer treated as
        // HLS — extractors that need HLS treatment must set
        // `format.ext = "hls"` explicitly.
        assert!(!url_is_hls("https://cdn/hls/abc123.mp4"));
    }

    #[test]
    fn accepts_m3u8_path() {
        assert!(url_is_hls("https://host/master.m3u8"));
    }

    #[test]
    fn ext_hls_short_circuits_when_url_has_no_extension() {
        // Contract: extractors that emit HLS without an `.m3u8` URL
        // (e.g. RedTube's KVS-style API output, formats.rs:240) set
        // `format.ext = "hls"`. The short-circuit MUST treat that as HLS
        // regardless of URL shape.
        assert!(is_hls_decision(
            "hls",
            "https://cdn.example.com/no-extension/abc123"
        ));
    }
}

#[cfg(test)]
mod resolve_timeout_tests {
    use super::resolve_hls_head_probe_timeout;
    use rdlp_types::Config;
    use std::time::Duration;

    #[test]
    fn head_probe_timeout_uses_default_when_none() {
        let c = Config {
            hls_head_probe_timeout: None,
            ..Config::default()
        };
        assert_eq!(resolve_hls_head_probe_timeout(&c), Duration::from_secs(5));
    }

    #[test]
    fn head_probe_timeout_uses_override_when_some() {
        let c = Config {
            hls_head_probe_timeout: Some(2),
            ..Config::default()
        };
        assert_eq!(resolve_hls_head_probe_timeout(&c), Duration::from_secs(2));
    }
}
