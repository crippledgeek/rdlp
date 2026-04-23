//! Megacloud / Rapid-Cloud embed source extraction.
//!
//! Resolves actual HLS video URLs from the Megacloud/Rapid-Cloud embed pages
//! that 9anime's servers redirect to.
//!
//! ## Extraction Flow
//!
//! 1. Extract source ID from the embed URL
//! 2. Fetch the megacloud decryption key from the keys repository
//! 3. Extract the client key from the v3 embed page HTML
//! 4. Call `getSources` API (v3 with client key, v2 fallback)
//! 5. Decrypt the response if encrypted (custom 3-layer cipher)
//!
//! ## References
//!
//! Based on the `extract5` / `extract3` methods from the aniwatch project's
//! megacloud extractor.

pub mod cipher;
mod client_key;

use anyhow::Context as _;
use log::debug;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use regex::Regex;
use std::sync::LazyLock;

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
///
/// Captures the source ID from paths like:
/// - `/embed-2/v2/e-1/{id}?z=`
/// - `/e-1/{id}?z=`
static SOURCE_ID_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/(?:e-\d+|embed-\d+/v\d+/e-\d+)/([^?]+)").expect("Valid source ID pattern")
});

/// Pattern to extract the embed base path (scheme + host + path before the
/// source ID). Used to construct the getSources URL on the same domain.
///
/// Captures: `https://rapid-cloud.co/embed-2/v2/e-1`
static EMBED_BASE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(https?://[^/]+/embed-\d+/v\d+/e-\d+)/[^?]+").expect("Valid embed base pattern")
});

/// The Megacloud API domain — used as fallback when the embed domain's
/// getSources endpoint doesn't work.
const MEGACLOUD_API: &str = "https://megacloud.blog";

/// URL for the externally-maintained decryption keys.
///
/// Pinned to a specific commit SHA to prevent supply-chain attacks via
/// branch-head updates. To update: fetch the latest commit SHA with
/// `curl -s https://api.github.com/repos/yogesh-hacker/MegacloudKeys/commits/main | jq -r .sha`
/// and replace the SHA in the URL below.
const KEYS_URL: &str = "https://raw.githubusercontent.com/yogesh-hacker/MegacloudKeys/514f571a035d8700f6b3ee3531897c9706fbc5cb/keys.json";

/// Extract video sources from a Megacloud embed URL.
///
/// This is the main entry point for Megacloud source resolution.
/// Tries multiple strategies in order:
///
/// 1. **Same-domain**: Construct getSources URL from the embed URL itself
///    (works for `rapid-cloud.co` embeds — usually returns unencrypted sources)
/// 2. **v3 megacloud.blog**: Client key extraction + custom 3-layer cipher
///    (works for `megacloud.blog` embeds)
pub async fn extract_sources(embed_url: &str, ctx: &ExtractionContext) -> Result<MegacloudSources> {
    let source_id = extract_source_id(embed_url).ok_or_else(|| RdlpError::Extraction {
        message: format!("Could not extract source ID from embed URL: {embed_url}"),
        url: Some(embed_url.to_string()),
    })?;
    debug!(source_id:%; "Extracted Megacloud source ID");

    // Strategy 1: getSources on the same domain/path as the embed URL
    if let Some(base) = extract_embed_base(embed_url) {
        let url = format!("{base}/getSources?id={source_id}");
        match fetch_get_sources(&url, embed_url, ctx).await {
            Ok(json) => match parse_sources_response(&json, None, None) {
                Ok(sources) if !sources.sources.is_empty() => {
                    debug!("Resolved sources via same-domain getSources");
                    return Ok(sources);
                }
                Ok(_) => debug!("Same-domain getSources returned empty sources"),
                Err(e) => debug!("Same-domain parse failed: {e}"),
            },
            Err(e) => debug!("Same-domain getSources failed: {e}"),
        }
    }

    // Strategy 2: v3 endpoint on megacloud.blog with client key + decryption
    match try_extract_v3(&source_id, embed_url, ctx).await {
        Ok(sources) => return Ok(sources),
        Err(e) => {
            debug!("v3 extraction failed: {e}");
        }
    }

    Err(RdlpError::Extraction {
        message: "All Megacloud extraction strategies failed".to_string(),
        url: Some(embed_url.to_string()),
    })
}

/// Strategy 2: v3 endpoint on megacloud.blog with client key + custom cipher.
async fn try_extract_v3(
    source_id: &str,
    embed_url: &str,
    ctx: &ExtractionContext,
) -> Result<MegacloudSources> {
    let megacloud_key = fetch_megacloud_key(ctx).await?;
    debug!("Fetched megacloud key");

    let client_key = client_key::extract_client_key(source_id, ctx).await?;
    debug!("Extracted client key");

    let url = format!("{MEGACLOUD_API}/embed-2/v3/e-1/getSources?id={source_id}&_k={client_key}");
    let json = fetch_get_sources(&url, embed_url, ctx).await?;

    parse_sources_response(&json, Some(&client_key), Some(&megacloud_key))
}

/// Extract the source ID from a Megacloud/Rapid-Cloud embed URL.
fn extract_source_id(embed_url: &str) -> Option<String> {
    SOURCE_ID_PATTERN
        .captures(embed_url)
        .map(|caps| caps[1].to_string())
}

/// Extract the embed base URL (scheme + host + path prefix).
///
/// For `https://rapid-cloud.co/embed-2/v2/e-1/ABC?z=` returns
/// `https://rapid-cloud.co/embed-2/v2/e-1`.
fn extract_embed_base(embed_url: &str) -> Option<String> {
    EMBED_BASE_PATTERN
        .captures(embed_url)
        .map(|caps| caps[1].to_string())
}

// ── Megacloud key ────────────────────────────────────────────────────

/// Fetch the megacloud decryption key from the keys repository.
async fn fetch_megacloud_key(ctx: &ExtractionContext) -> Result<String> {
    let response = ctx
        .http_client
        .get(KEYS_URL)
        .send()
        .await
        .map_err(|e| RdlpError::Network {
            message: format!("Failed to fetch megacloud keys: {e}"),
            url: Some(KEYS_URL.to_string()),
        })?;

    let json: serde_json::Value = response.json().await.map_err(|e| RdlpError::Extraction {
        message: format!("Failed to parse megacloud keys: {e}"),
        url: Some(KEYS_URL.to_string()),
    })?;

    let key = json["mega"].as_str().ok_or_else(|| RdlpError::Extraction {
        message: "No 'mega' field in megacloud keys response".to_string(),
        url: Some(KEYS_URL.to_string()),
    })?;

    // Validate key: non-empty, ASCII-only printable characters, reasonable length
    if key.is_empty() {
        return Err(RdlpError::Extraction {
            message: "Megacloud key is empty".to_string(),
            url: Some(KEYS_URL.to_string()),
        });
    }
    if !key.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return Err(RdlpError::Extraction {
            message: "Megacloud key contains non-ASCII or non-printable characters".to_string(),
            url: Some(KEYS_URL.to_string()),
        });
    }
    if key.len() > 512 {
        return Err(RdlpError::Extraction {
            message: format!(
                "Megacloud key length {} exceeds maximum of 512 bytes",
                key.len()
            ),
            url: Some(KEYS_URL.to_string()),
        });
    }

    Ok(key.to_string())
}

// ── getSources API ───────────────────────────────────────────────────

/// Fetch sources from the getSources API endpoint.
async fn fetch_get_sources(
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
        .map_err(|e| RdlpError::Network {
            message: format!("Failed to fetch getSources: {e}"),
            url: Some(url.to_string()),
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|e| RdlpError::Network {
        message: format!("Failed to read getSources body: {e}"),
        url: Some(url.to_string()),
    })?;

    if !status.is_success() {
        return Err(RdlpError::Extraction {
            message: format!(
                "getSources returned HTTP {status}: {}",
                &body[..body.len().min(200)]
            ),
            url: Some(url.to_string()),
        });
    }

    debug!(body_len = body.len(); "getSources response received");

    serde_json::from_str(&body).map_err(|e| RdlpError::Extraction {
        message: format!(
            "Failed to parse getSources JSON: {e}. Body: {}",
            &body[..body.len().min(200)]
        ),
        url: Some(url.to_string()),
    })
}

// ── Response parsing ─────────────────────────────────────────────────

/// Parse the getSources response, decrypting if necessary.
fn parse_sources_response(
    json: &serde_json::Value,
    client_key: Option<&str>,
    megacloud_key: Option<&str>,
) -> Result<MegacloudSources> {
    parse_sources_response_impl(json, client_key, megacloud_key).map_err(|e| {
        RdlpError::Extraction {
            message: format!("{e:#}"),
            url: None,
        }
    })
}

fn parse_sources_response_impl(
    json: &serde_json::Value,
    client_key: Option<&str>,
    megacloud_key: Option<&str>,
) -> anyhow::Result<MegacloudSources> {
    let encrypted = json["encrypted"].as_bool().unwrap_or(false);

    let sources = if encrypted {
        let encrypted_str = json["sources"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("encrypted sources field is not a string"))?;

        match (client_key, megacloud_key) {
            (Some(ck), Some(mk)) => {
                debug!("Decrypting sources with custom cipher");
                let decrypted = cipher::decrypt_src(encrypted_str, ck, mk)?;
                let arr: Vec<serde_json::Value> = serde_json::from_str(&decrypted)
                    .context("failed to parse decrypted megacloud sources JSON")?;
                parse_source_array_from_values(&arr)
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "sources are encrypted but no decryption keys available"
                ));
            }
        }
    } else {
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
fn parse_source_array(sources_json: &serde_json::Value) -> anyhow::Result<Vec<VideoSource>> {
    let arr = sources_json
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("sources field is not a JSON array"))?;

    Ok(parse_source_array_from_values(arr))
}

/// Parse video sources from a slice of JSON values.
fn parse_source_array_from_values(arr: &[serde_json::Value]) -> Vec<VideoSource> {
    arr.iter()
        .filter_map(|s| {
            let url = s["file"].as_str()?.to_string();
            let source_type = s["type"].as_str().unwrap_or("hls").to_string();
            Some(VideoSource { url, source_type })
        })
        .collect()
}

/// Parse subtitle tracks from JSON.
fn parse_tracks(tracks_json: &serde_json::Value) -> Vec<SubtitleTrack> {
    let Some(arr) = tracks_json.as_array() else {
        debug!("No tracks array in response");
        return Vec::new();
    };

    debug!(total = arr.len(); "Raw tracks in API response");

    // Log all track kinds for diagnostics
    for t in arr {
        let kind = t["kind"].as_str().unwrap_or("(none)");
        let label = t["label"].as_str().unwrap_or("(none)");
        debug!(kind, label; "Track entry");
    }

    let result: Vec<SubtitleTrack> = arr
        .iter()
        .filter_map(|t| {
            let kind = t["kind"].as_str().unwrap_or("");
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
        .collect();

    debug!(
        subtitle_tracks = result.len();
        "Filtered subtitle tracks (captions/subtitles only)"
    );

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_embed_base() {
        assert_eq!(
            extract_embed_base("https://rapid-cloud.co/embed-2/v2/e-1/8ZNLCBE5bdhP?z="),
            Some("https://rapid-cloud.co/embed-2/v2/e-1".to_string())
        );
        assert_eq!(
            extract_embed_base("https://megacloud.blog/embed-2/v3/e-1/ABC123?z="),
            Some("https://megacloud.blog/embed-2/v3/e-1".to_string())
        );
        assert_eq!(extract_embed_base("https://example.com/video"), None);
    }

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
    }

    #[test]
    fn test_parse_tracks() {
        let json = serde_json::json!([
            {"file": "https://example.com/en.vtt", "label": "English", "kind": "captions", "default": true},
            {"file": "https://example.com/thumb.vtt", "label": "thumbnails", "kind": "thumbnails"},
            {"file": "https://example.com/ja.vtt", "label": "Japanese", "kind": "captions"},
        ]);
        let tracks = parse_tracks(&json);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].label, "English");
        assert!(tracks[0].is_default);
        assert_eq!(tracks[1].label, "Japanese");
    }

    #[test]
    fn test_parse_tracks_empty() {
        let tracks = parse_tracks(&serde_json::Value::Null);
        assert!(tracks.is_empty());
    }

    #[test]
    fn test_parse_unencrypted_response() {
        let json = serde_json::json!({
            "sources": [{"file": "https://example.com/master.m3u8", "type": "hls"}],
            "tracks": [{"file": "https://example.com/en.vtt", "label": "English", "kind": "captions"}],
            "intro": {"start": 0.0, "end": 90.0},
            "outro": {"start": 1300.0, "end": 1400.0},
            "encrypted": false,
        });
        let result = parse_sources_response(&json, None, None).unwrap();
        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.tracks.len(), 1);
        assert_eq!(result.intro, Some((0.0, 90.0)));
        assert_eq!(result.outro, Some((1300.0, 1400.0)));
    }
}
