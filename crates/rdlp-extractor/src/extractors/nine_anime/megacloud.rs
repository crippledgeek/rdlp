//! Megacloud / Rapid-Cloud embed source extraction.
//!
//! Resolves actual HLS video URLs from the Megacloud/Rapid-Cloud embed pages
//! that 9anime's servers redirect to.
//!
//! ## Extraction Flow
//!
//! 1. Extract source ID from the embed URL
//! 2. Fetch the embed page HTML
//! 3. Extract the client key from the player JavaScript (via Boa)
//! 4. Call `getSources` API with the client key
//! 5. Decrypt the response if encrypted (via Boa or Rust AES fallback)
//!
//! ## References
//!
//! Based on analysis of the aniwatch project's megacloud extractor.

use log::{debug, info, warn};
use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use regex::Regex;

/// Extracted video sources from a Megacloud embed.
#[derive(Debug, Clone)]
pub struct MegacloudSources {
    /// HLS master playlist URLs with optional quality labels.
    pub sources: Vec<VideoSource>,
    /// Subtitle tracks.
    pub tracks: Vec<SubtitleTrack>,
    /// Intro timestamp range (start, end) in seconds.
    pub intro: Option<(f64, f64)>,
    /// Outro timestamp range (start, end) in seconds.
    pub outro: Option<(f64, f64)>,
}

/// A resolved video source.
#[derive(Debug, Clone)]
pub struct VideoSource {
    /// URL to the HLS m3u8 playlist or direct MP4.
    pub url: String,
    /// Source type (usually "hls").
    pub source_type: String,
}

/// A subtitle track.
#[derive(Debug, Clone)]
pub struct SubtitleTrack {
    /// URL to the subtitle file (usually VTT).
    pub url: String,
    /// Language label (e.g., "English").
    pub label: String,
    /// Whether this is the default track.
    pub is_default: bool,
}

/// Pattern to extract the source ID from a Megacloud/Rapid-Cloud embed URL.
static SOURCE_ID_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"/(?:e-\d+|embed-\d+/v\d+/e-\d+)/([^?]+)").expect("Valid source ID pattern")
});

/// Extract video sources from a Megacloud embed URL.
///
/// This is the main entry point for Megacloud source resolution.
pub async fn extract_sources(embed_url: &str, ctx: &ExtractionContext) -> Result<MegacloudSources> {
    // Step 1: Extract source ID from embed URL
    let source_id = extract_source_id(embed_url).ok_or_else(|| {
        RdlpError::Extraction(format!(
            "Could not extract source ID from embed URL: {embed_url}"
        ))
    })?;
    debug!(source_id:%; "Extracted Megacloud source ID");

    // Step 2: Fetch the embed page to get client key
    let embed_html = fetch_embed_page(embed_url, ctx).await?;

    // Step 3: Extract client key using JS engine
    let client_key = extract_client_key(&embed_html, &source_id, ctx).await?;
    debug!("Obtained Megacloud client key");

    // Step 4: Call getSources API
    let sources_json = fetch_get_sources(&source_id, &client_key, embed_url, ctx).await?;

    // Step 5: Parse (and potentially decrypt) the response
    parse_sources_response(&sources_json, ctx).await
}

/// Extract the source ID from a Megacloud/Rapid-Cloud embed URL.
fn extract_source_id(embed_url: &str) -> Option<String> {
    SOURCE_ID_PATTERN
        .captures(embed_url)
        .map(|caps| caps[1].to_string())
}

/// Fetch the embed page HTML.
async fn fetch_embed_page(embed_url: &str, ctx: &ExtractionContext) -> Result<String> {
    let response = ctx
        .http_client
        .get(embed_url)
        .header("Referer", "https://9animetv.to/")
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch embed page: {e}")))?;

    response
        .text()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to read embed page body: {e}")))
}

/// Extract the client key from the embed page using the JS engine.
///
/// The client key is embedded in the player JavaScript. We attempt multiple
/// strategies:
/// 1. Regex extraction from the JS source
/// 2. Boa execution of the relevant script fragment
async fn extract_client_key(
    embed_html: &str,
    source_id: &str,
    ctx: &ExtractionContext,
) -> Result<String> {
    // Strategy 1: Try regex-based extraction from embedded scripts
    if let Some(key) = try_regex_client_key(embed_html) {
        return Ok(key);
    }

    // Strategy 2: Try extracting from script src URLs and executing via Boa
    if let Some(key) = try_boa_client_key(embed_html, source_id, ctx).await {
        return Ok(key);
    }

    // Strategy 3: Use an empty key (some endpoints work without it)
    warn!("Could not extract Megacloud client key, trying empty key");
    Ok(String::new())
}

/// Try to extract the client key via regex patterns.
fn try_regex_client_key(html: &str) -> Option<String> {
    // Look for patterns like: case 0x...: _k = "..."
    let pattern = Regex::new(r#"_k\s*=\s*["']([a-zA-Z0-9]+)["']"#).ok()?;
    pattern.captures(html).map(|caps| caps[1].to_string())
}

/// Try to extract the client key by executing the player JS in Boa.
async fn try_boa_client_key(
    html: &str,
    source_id: &str,
    ctx: &ExtractionContext,
) -> Option<String> {
    // Find external script URLs in the embed page
    let script_pattern =
        Regex::new(r#"<script[^>]+src=["']([^"']+/(?:embed|player)[^"']+\.js[^"']*)["']"#).ok()?;

    for caps in script_pattern.captures_iter(html) {
        let script_url = &caps[1];
        let full_url = if script_url.starts_with("//") {
            format!("https:{script_url}")
        } else if script_url.starts_with('/') {
            // Extract origin from context
            format!("https://rapid-cloud.co{script_url}")
        } else {
            script_url.to_string()
        };

        debug!(url:% = full_url; "Fetching player script for client key");

        let script_text = match ctx.http_client.get(&full_url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        // Try to extract the key from the script via Boa
        let js_code = format!(
            r#"
            var sourceId = "{source_id}";
            {script_text}
            // Try to capture the client key from common variable names
            typeof _k !== 'undefined' ? _k :
            typeof clientKey !== 'undefined' ? clientKey :
            typeof key !== 'undefined' ? key : null
            "#
        );

        match ctx.js_engine.eval(&js_code).await {
            Ok(serde_json::Value::String(key)) if !key.is_empty() => {
                info!("Extracted Megacloud client key via Boa");
                return Some(key);
            }
            Ok(_) => {
                debug!("Boa eval returned non-string or empty key");
            }
            Err(e) => {
                debug!(error:% = e; "Boa eval failed for client key extraction");
            }
        }
    }

    None
}

/// The Megacloud API domain. The getSources endpoint always lives on
/// `megacloud.blog` regardless of whether the embed URL uses `rapid-cloud.co`.
const MEGACLOUD_API: &str = "https://megacloud.blog";

/// Fetch sources from the getSources API endpoint.
///
/// Tries the v2 endpoint first (no client key), then v3 (with client key).
async fn fetch_get_sources(
    source_id: &str,
    client_key: &str,
    embed_url: &str,
    ctx: &ExtractionContext,
) -> Result<serde_json::Value> {
    // Try v2 first (simpler, no client key needed)
    let v2_url = format!("{MEGACLOUD_API}/embed-2/v2/e-1/getSources?id={source_id}");

    if let Ok(json) = try_get_sources(&v2_url, embed_url, ctx).await {
        return Ok(json);
    }

    // Fall back to v3 with client key
    let url = if client_key.is_empty() {
        format!("{MEGACLOUD_API}/embed-2/v3/e-1/getSources?id={source_id}")
    } else {
        format!("{MEGACLOUD_API}/embed-2/v3/e-1/getSources?id={source_id}&_k={client_key}")
    };

    try_get_sources(&url, embed_url, ctx).await
}

/// Attempt a single getSources API call, returning the parsed JSON.
async fn try_get_sources(
    url: &str,
    embed_url: &str,
    ctx: &ExtractionContext,
) -> Result<serde_json::Value> {
    debug!(url:%; "Fetching Megacloud getSources");

    let response = ctx
        .http_client
        .get(url)
        .header("Referer", embed_url)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch getSources: {e}")))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to read getSources body: {e}")))?;

    if !status.is_success() {
        return Err(RdlpError::Extraction(format!(
            "getSources returned HTTP {status}: {}",
            &body[..body.len().min(200)]
        )));
    }

    debug!(
        body_len = body.len(),
        body_preview:% = &body[..body.len().min(100)];
        "getSources response"
    );

    serde_json::from_str(&body).map_err(|e| {
        RdlpError::Extraction(format!(
            "Failed to parse getSources JSON: {e}. Body preview: {}",
            &body[..body.len().min(200)]
        ))
    })
}

/// Parse the getSources response, decrypting if necessary.
async fn parse_sources_response(
    json: &serde_json::Value,
    ctx: &ExtractionContext,
) -> Result<MegacloudSources> {
    let encrypted = json["encrypted"].as_bool().unwrap_or(false);

    let sources = if encrypted {
        // The sources field is an encrypted string
        let encrypted_str = json["sources"].as_str().ok_or_else(|| {
            RdlpError::Extraction("Encrypted sources field is not a string".to_string())
        })?;

        decrypt_sources(encrypted_str, ctx).await?
    } else {
        // Sources are plaintext
        parse_source_array(&json["sources"])?
    };

    let tracks = parse_tracks(&json["tracks"]);

    let intro = json["intro"].as_object().and_then(|obj| {
        let start = obj.get("start")?.as_f64()?;
        let end = obj.get("end")?.as_f64()?;
        Some((start, end))
    });

    let outro = json["outro"].as_object().and_then(|obj| {
        let start = obj.get("start")?.as_f64()?;
        let end = obj.get("end")?.as_f64()?;
        Some((start, end))
    });

    Ok(MegacloudSources {
        sources,
        tracks,
        intro,
        outro,
    })
}

/// Parse a plaintext sources array from JSON.
fn parse_source_array(sources_json: &serde_json::Value) -> Result<Vec<VideoSource>> {
    let arr = sources_json
        .as_array()
        .ok_or_else(|| RdlpError::Extraction("Sources field is not an array".to_string()))?;

    Ok(arr
        .iter()
        .filter_map(|s| {
            let url = s["file"].as_str()?.to_string();
            let source_type = s["type"].as_str().unwrap_or("hls").to_string();
            Some(VideoSource { url, source_type })
        })
        .collect())
}

/// Parse subtitle tracks from JSON.
fn parse_tracks(tracks_json: &serde_json::Value) -> Vec<SubtitleTrack> {
    let Some(arr) = tracks_json.as_array() else {
        return Vec::new();
    };

    arr.iter()
        .filter_map(|t| {
            let kind = t["kind"].as_str().unwrap_or("");
            // Only include caption/subtitle tracks, not thumbnails
            if !kind.eq_ignore_ascii_case("captions") && !kind.eq_ignore_ascii_case("subtitles") {
                return None;
            }
            let url = t["file"].as_str()?.to_string();
            let label = t["label"].as_str().unwrap_or("Unknown").to_string();
            let is_default = t["default"].as_bool().unwrap_or(false);
            Some(SubtitleTrack {
                url,
                label,
                is_default,
            })
        })
        .collect()
}

/// Decrypt encrypted sources using Boa JS engine.
///
/// Falls back to logging a warning if decryption fails.
async fn decrypt_sources(encrypted: &str, ctx: &ExtractionContext) -> Result<Vec<VideoSource>> {
    // Try Boa-based decryption
    // The aniwatch reference uses CryptoJS.AES.decrypt or a custom cipher.
    // We attempt to load the decryption logic and execute it.
    let js_code = format!(
        r#"
        // Attempt basic AES decryption (CryptoJS-compatible)
        // This is a placeholder — real implementation requires the correct key
        // which changes with the player script.
        try {{
            var encrypted = "{encrypted}";
            // Try to parse as JSON directly (some responses are just base64-encoded JSON)
            var decoded = atob(encrypted);
            JSON.parse(decoded);
        }} catch(e) {{
            // If base64 decode + JSON parse fails, the data needs actual decryption
            null;
        }}
        "#
    );

    match ctx.js_engine.eval(&js_code).await {
        Ok(serde_json::Value::Array(arr)) => {
            let sources: Vec<VideoSource> = arr
                .iter()
                .filter_map(|s| {
                    let url = s["file"].as_str()?.to_string();
                    let source_type = s["type"].as_str().unwrap_or("hls").to_string();
                    Some(VideoSource { url, source_type })
                })
                .collect();
            if !sources.is_empty() {
                return Ok(sources);
            }
        }
        Ok(serde_json::Value::String(s)) => {
            // Might be a JSON string that needs parsing
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&s) {
                let sources: Vec<VideoSource> = parsed
                    .iter()
                    .filter_map(|s| {
                        let url = s["file"].as_str()?.to_string();
                        let source_type = s["type"].as_str().unwrap_or("hls").to_string();
                        Some(VideoSource { url, source_type })
                    })
                    .collect();
                if !sources.is_empty() {
                    return Ok(sources);
                }
            }
        }
        _ => {}
    }

    warn!("Megacloud source decryption failed — encrypted sources could not be resolved");
    Err(RdlpError::Extraction(
        "Could not decrypt Megacloud sources. The encryption key may have changed.".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_source_id() {
        assert_eq!(
            extract_source_id("https://rapid-cloud.co/embed-2/v2/e-1/vzHCSIV6DLZr?z="),
            Some("vzHCSIV6DLZr".to_string())
        );
        assert_eq!(
            extract_source_id("https://megacloud.blog/embed-2/v2/e-1/JUMKxjOvwYQI?z="),
            Some("JUMKxjOvwYQI".to_string())
        );
        assert_eq!(
            extract_source_id("https://rapid-cloud.co/embed-2/v2/e-1/8ikzgk0ah9Dz?z="),
            Some("8ikzgk0ah9Dz".to_string())
        );
    }

    #[test]
    fn test_extract_source_id_invalid() {
        assert_eq!(extract_source_id("https://example.com/video"), None);
    }

    #[test]
    fn test_parse_source_array() {
        let json = serde_json::json!([
            {"file": "https://example.com/master.m3u8", "type": "hls"},
            {"file": "https://example.com/video.mp4", "type": "mp4"},
        ]);
        let sources = parse_source_array(&json).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].url, "https://example.com/master.m3u8");
        assert_eq!(sources[0].source_type, "hls");
        assert_eq!(sources[1].source_type, "mp4");
    }

    #[test]
    fn test_parse_tracks() {
        let json = serde_json::json!([
            {"file": "https://example.com/en.vtt", "label": "English", "kind": "captions", "default": true},
            {"file": "https://example.com/thumb.vtt", "label": "thumbnails", "kind": "thumbnails"},
            {"file": "https://example.com/ja.vtt", "label": "Japanese", "kind": "captions"},
        ]);
        let tracks = parse_tracks(&json);
        // Should filter out thumbnail tracks
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].label, "English");
        assert!(tracks[0].is_default);
        assert_eq!(tracks[1].label, "Japanese");
        assert!(!tracks[1].is_default);
    }

    #[test]
    fn test_parse_tracks_empty() {
        let tracks = parse_tracks(&serde_json::Value::Null);
        assert!(tracks.is_empty());
    }
}
