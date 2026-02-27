//! Format extraction for xHamster.
//!
//! Two code paths:
//! - **Modern**: `window.initials` JSON with `videoModel.sources` and
//!   `xplayerSettings.sources` (HLS + standard, encrypted URLs).
//! - **Legacy**: Regex scraping for `sources: {...}`, `file: "..."`, etc.

use std::collections::{HashMap, HashSet};

use log::debug;
use rdlp_core::Format;
use serde_json::Value;

use rdlp_core::JsEngine;

use crate::base::common::BaseExtractor;

use super::patterns;

/// Extract height from a quality string like "720p", "1080P", etc.
fn get_height(s: &str) -> Option<u32> {
    BaseExtractor::parse_quality_height(s)
}


/// Detect vcodec from URL by checking for `.av1.` or `.h264.` in the path.
fn detect_vcodec(url: &str) -> Option<&'static str> {
    [(".av1.", "av1"), (".h264.", "h264")]
        .iter()
        .find(|(pattern, _)| url.contains(pattern))
        .map(|(_, codec)| *codec)
}

/// Apply codec fixup to all formats (detect vcodec from URL patterns).
pub fn fixup_formats(formats: &mut [Format]) {
    for f in formats.iter_mut().filter(|f| f.vcodec.is_none()) {
        if let Some(vcodec) = detect_vcodec(&f.url) {
            f.vcodec = Some(vcodec.to_string());
        }
    }
}

/// Extract formats from `window.initials` JSON (modern layout).
///
/// Processes:
/// 1. `videoModel.sources` — direct URLs keyed by format type and quality
/// 2. `xplayerSettings.sources.hls` — encrypted HLS manifests
/// 3. `xplayerSettings.sources.standard` — encrypted standard video URLs
pub async fn extract_from_initials(
    initials: &Value,
    page_url: &str,
    js_engine: &dyn JsEngine,
    player_decrypt_js: Option<&str>,
) -> Vec<Format> {
    let mut formats = Vec::new();
    let mut seen_urls: HashSet<String> = HashSet::new();

    let video_model = match initials.get("videoModel") {
        Some(vm) => vm,
        None => {
            debug!("[XHamster] No videoModel in initials");
            return formats;
        }
    };

    let empty_map = serde_json::Map::new();
    let sources = video_model
        .get("sources")
        .and_then(|v| v.as_object())
        .unwrap_or(&empty_map);

    // Collect download sizes from sources.download
    let mut format_sizes: HashMap<String, f64> = HashMap::new();
    if let Some(download) = sources.get("download").and_then(|v| v.as_object()) {
        for (quality, format_dict) in download {
            if let Some(size) = format_dict.get("size").and_then(|v| v.as_f64()) {
                format_sizes.insert(quality.clone(), size);
            }
        }
    }

    // Skip videoModel.sources direct URLs — XHamster CDN blocks direct
    // MP4 downloads (403 Forbidden). Only xplayerSettings.sources URLs
    // (HLS + encrypted standard) are functional. File sizes from
    // sources.download are still used to annotate decrypted formats.
    debug!(
        "[XHamster] Skipping {} videoModel.sources direct URL key(s) (CDN-blocked)",
        sources.keys().filter(|k| *k != "download").count()
    );

    // Extract from xplayerSettings.sources
    // Also try xplayerSettings2 as a fallback
    let xplayer_sources_val = initials
        .pointer("/xplayerSettings/sources")
        .or_else(|| initials.pointer("/xplayerSettings2/sources"));

    if let Some(xplayer_sources) = xplayer_sources_val.and_then(|v| v.as_object()) {
        debug!(
            "[XHamster] Found xplayerSettings.sources with {} keys",
            xplayer_sources.len()
        );
        // HLS sources (encrypted)
        // Supports two layouts:
        //   Old: hls: {url: "...", fallback: "..."}
        //   New: hls: {av1: {url: "..."}, h264: {url: "...", fallback: "..."}}
        if let Some(hls) = xplayer_sources.get("hls").and_then(|v| v.as_object()) {
            // Collect all (format_id, encrypted_url) pairs from both layouts
            // IMPORTANT: Process top-level url/fallback FIRST (master playlists),
            // then codec-specific URLs. This ensures master playlists are preferred
            // during deduplication (they typically have complete segment lists).
            let mut hls_urls: Vec<(String, String)> = Vec::new();

            // First: top-level "url" and "fallback" keys (master playlists)
            for top_key in &["url", "fallback"] {
                if let Some(url_str) = hls.get(*top_key).and_then(|v| v.as_str())
                    && !url_str.is_empty()
                {
                    hls_urls.push((format!("hls-{top_key}"), url_str.to_string()));
                }
            }

            // Second: codec-specific nested objects (h264, av1, etc.)
            for (key, value) in hls {
                // Skip top-level keys already processed
                if key == "url" || key == "fallback" {
                    continue;
                }
                if let Some(codec_obj) = value.as_object() {
                    // New layout: codec-keyed objects like {url: "...", fallback: "..."}
                    for hls_key in &["url", "fallback"] {
                        if let Some(url_str) = codec_obj.get(*hls_key).and_then(|v| v.as_str())
                            && !url_str.is_empty()
                        {
                            hls_urls.push((format!("hls-{key}-{hls_key}"), url_str.to_string()));
                        }
                    }
                }
            }

            for (format_id, hls_url) in hls_urls {
                let Some(deciphered) =
                    super::js_extract::decipher_url_via_boa(&hls_url, js_engine, player_decrypt_js)
                        .await
                else {
                    debug!(format_id:?; "[XHamster] Failed to decipher HLS URL");
                    continue;
                };
                if !seen_urls.insert(deciphered.clone()) {
                    continue;
                }
                // HLS manifests will be expanded by detect_format_sizes
                let mut format = Format::new(
                    format_id,
                    deciphered,
                    "mp4",
                    rdlp_core::DownloadProtocol::M3u8Native,
                );
                format.http_headers = Some(referer_headers(page_url));
                formats.push(format);
            }
        }

        // Standard sources (encrypted)
        if let Some(standard) = xplayer_sources.get("standard").and_then(|v| v.as_object()) {
            for (identifier, formats_list) in standard {
                let Some(list) = formats_list.as_array() else {
                    continue;
                };
                for standard_format in list {
                    let Some(std_obj) = standard_format.as_object() else {
                        continue;
                    };
                    for std_key in &["url", "fallback"] {
                        let Some(std_url) = std_obj.get(*std_key).and_then(|v| v.as_str()) else {
                            continue;
                        };
                        if std_url.is_empty() {
                            continue;
                        }

                        // Extract quality as a displayable string from "quality" (string or int)
                        // or "label" as fallback.
                        let quality_str = std_obj
                            .get("quality")
                            .and_then(|v| {
                                v.as_str()
                                    .map(|s| s.to_string())
                                    .or_else(|| v.as_i64().map(|i| i.to_string()))
                            })
                            .or_else(|| {
                                std_obj
                                    .get("label")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                            })
                            .unwrap_or_default();

                        let format_id = if quality_str.is_empty() {
                            identifier.clone()
                        } else {
                            format!("{identifier}-{quality_str}")
                        };

                        let Some(deciphered) = super::js_extract::decipher_url_via_boa(
                            std_url,
                            js_engine,
                            player_decrypt_js,
                        )
                        .await
                        else {
                            debug!(format_id:?; "[XHamster] Failed to decipher standard URL");
                            continue;
                        };

                        // Only keep HLS URLs from the standard section.
                        // Direct MP4 URLs (video-h.xhcdn.com/key=...) are
                        // CDN-blocked (403) even after decryption. HLS URLs
                        // that appear in the standard section (media=hls4)
                        // are the only ones that work.
                        let is_hls = deciphered.contains("m3u8")
                            || deciphered.contains("media=hls");
                        if !is_hls {
                            debug!(
                                "[XHamster] Skipping standard direct URL {} (CDN-blocked)",
                                format_id
                            );
                            continue;
                        }

                        if !seen_urls.insert(deciphered.clone()) {
                            continue;
                        }

                        let mut format = Format::new(
                            format_id,
                            deciphered,
                            "mp4",
                            rdlp_core::DownloadProtocol::M3u8Native,
                        );
                        format.http_headers = Some(referer_headers(page_url));
                        formats.push(format);
                    }
                }
            }
        }
    } else {
        debug!("[XHamster] No xplayerSettings.sources found");
    }

    debug!("[XHamster] Total formats extracted: {}", formats.len());
    fixup_formats(&mut formats);
    formats
}

/// Extract formats from legacy HTML page (fallback).
pub fn extract_from_legacy(webpage: &str) -> Vec<Format> {
    let mut formats = Vec::new();
    let mut seen_urls: HashSet<String> = HashSet::new();

    // Strategy 1: sources: {...} JS object
    if let Some(sources) = patterns::LEGACY_SOURCES_PATTERN
        .captures(webpage)
        .and_then(|caps| caps.get(1))
        .and_then(|json_str| serde_json::from_str::<Value>(json_str.as_str()).ok())
        && let Some(obj) = sources.as_object()
    {
        for (format_id, url_val) in obj {
            let Some(url) = url_val.as_str().filter(|u| !u.is_empty()) else {
                continue;
            };
            if !seen_urls.insert(url.to_string()) {
                continue;
            }
            let height = get_height(format_id);
            formats.push(BaseExtractor::build_format(
                format_id.clone(),
                url.to_string(),
                "mp4".to_string(),
                height,
            ));
        }
    }

    // Strategy 2: file: "url", mp4Thumb, <video file="url">
    let url_patterns = [
        &patterns::LEGACY_FILE_PATTERN,
        &patterns::LEGACY_MP4_THUMB_PATTERN,
        &patterns::LEGACY_VIDEO_FILE_PATTERN,
    ];

    for pattern in &url_patterns {
        if let Some(url) = pattern
            .captures(webpage)
            .and_then(|caps| caps.name("url"))
            .map(|m| m.as_str())
            .filter(|u| !u.is_empty())
            && seen_urls.insert(url.to_string())
        {
            formats.push(Format::new(
                "video",
                url,
                "mp4",
                rdlp_core::DownloadProtocol::Https,
            ));
        }
    }

    fixup_formats(&mut formats);
    formats
}

/// Create HTTP headers map matching browser request patterns.
///
/// XHamster CDNs reject requests missing modern browser headers.
/// We send Referer, Origin, and Accept to pass anti-hotlinking checks.
fn referer_headers(page_url: &str) -> HashMap<String, String> {
    // Extract origin (scheme + host) from page URL
    let origin = page_url
        .find("://")
        .and_then(|scheme_end| {
            let host_start = scheme_end + 3;
            page_url[host_start..]
                .find('/')
                .map(|slash| &page_url[..host_start + slash])
        })
        .unwrap_or(page_url);

    HashMap::from([
        ("Referer".to_string(), page_url.to_string()),
        ("Origin".to_string(), origin.to_string()),
        ("Accept".to_string(), "*/*".to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_height() {
        assert_eq!(get_height("720p"), Some(720));
        assert_eq!(get_height("1080P"), Some(1080));
        assert_eq!(get_height("480"), Some(480));
        assert_eq!(get_height("hd"), None);
    }

    #[test]
    fn test_detect_vcodec() {
        assert_eq!(
            detect_vcodec("https://example.com/video.h264.mp4"),
            Some("h264")
        );
        assert_eq!(
            detect_vcodec("https://example.com/video.av1.mp4"),
            Some("av1")
        );
        assert_eq!(detect_vcodec("https://example.com/video.mp4"), None);
    }

    /// Encrypt a plaintext URL into the hex format that `decipher_url_via_boa` can decode.
    fn encrypt_url(plaintext: &str) -> String {
        use rdlp_crypto::prng::ByteGenerator;
        let algo_id: u8 = 1;
        let seed: i32 = 42;
        let mut rng = ByteGenerator::new(algo_id, seed).unwrap();
        let seed_bytes = seed.to_le_bytes();
        let mut hex_bytes = vec![algo_id];
        hex_bytes.extend_from_slice(&seed_bytes);
        for byte in plaintext.bytes() {
            hex_bytes.push(byte ^ rng.next_byte());
        }
        hex::encode(hex_bytes)
    }

    #[tokio::test]
    async fn test_extract_from_initials_basic() {
        use rdlp_jsinterp::BoaJsEngine;
        let engine = BoaJsEngine::new();
        // xplayerSettings.sources.hls with encrypted HLS URLs (the only
        // functional path — both videoModel.sources and standard direct
        // URLs are CDN-blocked).
        let enc_720 = encrypt_url("https://example.com/media=hls4/720.m3u8");
        let enc_1080 = encrypt_url("https://example.com/media=hls4/1080.m3u8");
        let initials = serde_json::json!({
            "videoModel": {
                "sources": {
                    "download": {
                        "720p": {"size": 50000000.0},
                        "1080p": {"size": 100000000.0}
                    }
                }
            },
            "xplayerSettings": {
                "sources": {
                    "hls": {
                        "url": enc_720,
                        "fallback": enc_1080
                    }
                }
            }
        });

        let formats = extract_from_initials(
            &initials,
            "https://xhamster.com/videos/test-123",
            &engine,
            None,
        )
        .await;
        assert_eq!(formats.len(), 2);
        assert!(formats.iter().any(|f| f.url.contains("720.m3u8")));
        assert!(formats.iter().any(|f| f.url.contains("1080.m3u8")));
    }

    #[test]
    fn test_extract_from_legacy() {
        let webpage = r#"
            var playerConfig = {
                sources: {"720": "https://example.com/720.mp4", "1080": "https://example.com/1080.mp4"},
                title: "Test"
            };
        "#;

        let formats = extract_from_legacy(webpage);
        assert_eq!(formats.len(), 2);
    }

    #[test]
    fn test_referer_headers() {
        let headers = referer_headers("https://xhamster.com/videos/test-123");
        assert_eq!(
            headers.get("Referer").unwrap(),
            "https://xhamster.com/videos/test-123"
        );
        assert_eq!(headers.get("Origin").unwrap(), "https://xhamster.com");
        assert_eq!(headers.get("Accept").unwrap(), "*/*");
    }

    #[tokio::test]
    async fn test_extract_from_initials_dedup() {
        use rdlp_jsinterp::BoaJsEngine;
        let engine = BoaJsEngine::new();
        // Same encrypted HLS URL in hls url and fallback → should dedup to 1
        let enc = encrypt_url("https://example.com/media=hls4/video.m3u8");
        let initials = serde_json::json!({
            "videoModel": { "sources": {} },
            "xplayerSettings": {
                "sources": {
                    "hls": {
                        "url": enc,
                        "fallback": enc
                    }
                }
            }
        });

        let formats = extract_from_initials(
            &initials,
            "https://xhamster.com/videos/test-123",
            &engine,
            None,
        )
        .await;
        // Same URL should be deduped
        assert_eq!(formats.len(), 1);
    }

    #[tokio::test]
    async fn test_extract_from_initials_skips_direct_urls() {
        use rdlp_jsinterp::BoaJsEngine;
        let engine = BoaJsEngine::new();
        // videoModel.sources direct URLs are CDN-blocked and should be skipped
        let initials = serde_json::json!({
            "videoModel": {
                "sources": {
                    "mp4": {
                        "720p": "https://example.com/720.mp4",
                        "1080p": "https://example.com/1080.mp4"
                    }
                }
            }
        });

        let formats = extract_from_initials(
            &initials,
            "https://xhamster.com/videos/test-123",
            &engine,
            None,
        )
        .await;
        assert_eq!(formats.len(), 0, "Direct URLs should be skipped (CDN-blocked)");
    }

}
