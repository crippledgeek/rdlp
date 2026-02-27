//! Boa-based JS extraction module for XHamster.
//!
//! Provides three public functions for boa-based extraction:
//! - [`extract_initials_via_boa`] — evaluates `window.initials` via boa
//! - [`find_player_script_urls`] — discovers player JS URLs from HTML
//! - [`decipher_url_via_boa`] — decrypts an encrypted URL using boa
//!
//! And one async helper:
//! - [`fetch_player_js`] — downloads player JS bundle from discovered URLs

use log::debug;
use rdlp_core::JsEngine;
use regex::Regex;
use std::sync::LazyLock;

/// Bundled JS decryption code (port of rdlp-crypto PRNG algorithms).
const BUNDLED_DECRYPT_JS: &str = include_str!("decrypt.js");

/// Pattern to find script blocks that assign window.initials.
static INITIALS_SCRIPT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<script[^>]*>\s*(window\.initials\s*=\s*\{.+?\})\s*;?\s*</script>")
        .expect("Valid initials script pattern")
});

/// Pattern to find player script URLs (matches xplayer or player in src).
static PLAYER_SCRIPT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<script[^>]+src=["']([^"']*(?:xplayer|player)[^"']*)["']"#)
        .expect("Valid player script pattern")
});

/// Known decryption function signatures in player JS bundles.
pub(crate) static DECRYPT_FUNC_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)function\s+(\w+)\s*\([^)]*\)\s*\{[^}]*(?:1664525|0x85ebca77|charCodeAt)[^}]*\}",
    )
    .expect("Valid decrypt function pattern")
});

/// Evaluate `window.initials` from the page HTML using boa.
///
/// Finds the `<script>` block containing `window.initials =`, evaluates it
/// in boa with `JSON.stringify(window.initials)` appended, and parses the
/// resulting JSON back in Rust.
///
/// Returns `None` if the script block is not found or boa eval fails.
pub async fn extract_initials_via_boa(
    webpage: &str,
    js_engine: &dyn JsEngine,
) -> Option<serde_json::Value> {
    let script_body = INITIALS_SCRIPT_PATTERN
        .captures(webpage)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())?;

    // Wrap: execute the assignment, then stringify the result
    let code = format!("{script_body};\nJSON.stringify(window.initials)");

    match js_engine.eval(&code).await {
        Ok(json_val) => {
            // boa returns the JSON.stringify result as a JSON string value
            let json_str = json_val.as_str()?;
            serde_json::from_str(json_str)
                .inspect_err(|e| debug!("[XHamster] Boa initials JSON parse failed: {e}"))
                .ok()
        }
        Err(e) => {
            debug!("[XHamster] Boa eval failed for window.initials: {e}");
            None
        }
    }
}

/// Scan page HTML for `<script src="...">` tags matching player patterns.
///
/// Returns candidate URLs that contain "xplayer" or "player" in the src attribute.
pub fn find_player_script_urls(webpage: &str) -> Vec<String> {
    PLAYER_SCRIPT_PATTERN
        .captures_iter(webpage)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Decipher an encrypted XHamster URL using boa.
///
/// If `player_decrypt_js` is provided, tries to find and call the site's own
/// decryption function first. Falls back to the bundled `decrypt.js` (loaded
/// via `include_str!`). Returns the decrypted URL or `None`.
pub async fn decipher_url_via_boa(
    encrypted_url: &str,
    js_engine: &dyn JsEngine,
    player_decrypt_js: Option<&str>,
) -> Option<String> {
    // Try site's own decryption JS first
    if let Some(player_js) = player_decrypt_js {
        if let Some(result) = try_player_decrypt(encrypted_url, js_engine, player_js).await {
            return Some(result);
        }
        debug!("[XHamster] Player JS decryption failed, falling back to bundled JS");
    }

    // Fall back to bundled decrypt.js
    try_bundled_decrypt(encrypted_url, js_engine).await
}

/// Fetch player JS from the discovered script URLs.
///
/// Tries each URL in order, returning the first response body that contains
/// known decryption-related constants. Returns `None` if no suitable JS is found.
pub async fn fetch_player_js(
    script_urls: &[String],
    http_client: &reqwest::Client,
    page_url: &str,
) -> Option<String> {
    for url in script_urls {
        let full_url = if url.starts_with("//") {
            format!("https:{url}")
        } else if url.starts_with('/') {
            // Relative URL — construct from page domain
            if let Ok(parsed) = url::Url::parse(page_url) {
                format!(
                    "{}://{}{}",
                    parsed.scheme(),
                    parsed.host_str().unwrap_or(""),
                    url
                )
            } else {
                continue;
            }
        } else {
            url.clone()
        };

        match http_client
            .get(&full_url)
            .header("Referer", page_url)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    // Verify it contains decryption-related code
                    if body.contains("1664525")
                        || body.contains("0x85ebca77")
                        || body.contains("charCodeAt")
                    {
                        debug!("[XHamster] Found player JS with decrypt code: {full_url}");
                        return Some(body);
                    }
                }
            }
            Ok(resp) => {
                debug!(
                    "[XHamster] Player JS fetch returned {}: {full_url}",
                    resp.status()
                );
            }
            Err(e) => {
                debug!("[XHamster] Player JS fetch failed: {e}");
            }
        }
    }
    None
}

/// Try decryption using the site's own player JS bundle.
async fn try_player_decrypt(
    encrypted_url: &str,
    js_engine: &dyn JsEngine,
    player_js: &str,
) -> Option<String> {
    // Find the decryption function name in the player bundle
    let func_name = DECRYPT_FUNC_PATTERN
        .captures(player_js)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())?;

    let code = format!("{player_js}\n{func_name}({encrypted_url:?})");

    match js_engine.eval(&code).await {
        Ok(val) => {
            if val.is_null() {
                None
            } else {
                val.as_str().map(|s| s.to_string())
            }
        }
        Err(e) => {
            debug!("[XHamster] Player decrypt eval failed: {e}");
            None
        }
    }
}

/// Try decryption using the bundled JS port of the 7 PRNG algorithms.
pub(crate) async fn try_bundled_decrypt(
    encrypted_url: &str,
    js_engine: &dyn JsEngine,
) -> Option<String> {
    let code = format!("{BUNDLED_DECRYPT_JS}\ndecipherFormatUrl({encrypted_url:?})");

    match js_engine.eval(&code).await {
        Ok(val) => {
            if val.is_null() {
                None
            } else {
                val.as_str().map(|s| s.to_string())
            }
        }
        Err(e) => {
            debug!("[XHamster] Bundled decrypt eval failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_crypto::xhamster::decipher_format_url as rust_decipher;
    use rdlp_jsinterp::BoaJsEngine;

    /// Helper: encrypt a plaintext string with a given algo+seed using Rust,
    /// producing a hex string that both Rust and JS can decipher.
    fn encrypt_test_vector(algo_id: u8, seed: i32, plaintext: &str) -> String {
        use rdlp_crypto::prng::ByteGenerator;
        let mut rng = ByteGenerator::new(algo_id, seed).unwrap();
        let seed_bytes = seed.to_le_bytes();
        let mut hex_bytes = vec![algo_id];
        hex_bytes.extend_from_slice(&seed_bytes);
        for byte in plaintext.bytes() {
            hex_bytes.push(byte ^ rng.next_byte());
        }
        hex::encode(hex_bytes)
    }

    // =========================================================================
    // Cross-validation: Rust vs JS parity
    // =========================================================================

    #[tokio::test]
    async fn test_rust_js_parity_algo_1() {
        let engine = BoaJsEngine::new();
        let hex = encrypt_test_vector(1, 42, "https://cdn.example.com/video.mp4");
        let rust_result = rust_decipher(&hex).unwrap();
        let js_result = try_bundled_decrypt(&hex, &engine).await.unwrap();
        assert_eq!(rust_result, js_result, "Algo 1 (LCG) parity failed");
    }

    #[tokio::test]
    async fn test_rust_js_parity_algo_2() {
        let engine = BoaJsEngine::new();
        let hex = encrypt_test_vector(2, 42, "https://cdn.example.com/video.mp4");
        let rust_result = rust_decipher(&hex).unwrap();
        let js_result = try_bundled_decrypt(&hex, &engine).await.unwrap();
        assert_eq!(rust_result, js_result, "Algo 2 (Xorshift32) parity failed");
    }

    #[tokio::test]
    async fn test_rust_js_parity_algo_3() {
        let engine = BoaJsEngine::new();
        let hex = encrypt_test_vector(3, 42, "https://cdn.example.com/video.mp4");
        let rust_result = rust_decipher(&hex).unwrap();
        let js_result = try_bundled_decrypt(&hex, &engine).await.unwrap();
        assert_eq!(rust_result, js_result, "Algo 3 (Weyl+fmix32) parity failed");
    }

    #[tokio::test]
    async fn test_rust_js_parity_algo_4() {
        let engine = BoaJsEngine::new();
        let hex = encrypt_test_vector(4, 42, "https://cdn.example.com/video.mp4");
        let rust_result = rust_decipher(&hex).unwrap();
        let js_result = try_bundled_decrypt(&hex, &engine).await.unwrap();
        assert_eq!(rust_result, js_result, "Algo 4 (Weyl+ROL7) parity failed");
    }

    #[tokio::test]
    async fn test_rust_js_parity_algo_5() {
        let engine = BoaJsEngine::new();
        let hex = encrypt_test_vector(5, 42, "https://cdn.example.com/video.mp4");
        let rust_result = rust_decipher(&hex).unwrap();
        let js_result = try_bundled_decrypt(&hex, &engine).await.unwrap();
        assert_eq!(
            rust_result, js_result,
            "Algo 5 (Xorshift+add) parity failed"
        );
    }

    #[tokio::test]
    async fn test_rust_js_parity_algo_6() {
        let engine = BoaJsEngine::new();
        let hex = encrypt_test_vector(6, 42, "https://cdn.example.com/video.mp4");
        let rust_result = rust_decipher(&hex).unwrap();
        let js_result = try_bundled_decrypt(&hex, &engine).await.unwrap();
        assert_eq!(rust_result, js_result, "Algo 6 (LCG+PCG) parity failed");
    }

    #[tokio::test]
    async fn test_rust_js_parity_algo_7() {
        let engine = BoaJsEngine::new();
        let hex = encrypt_test_vector(7, 42, "https://cdn.example.com/video.mp4");
        let rust_result = rust_decipher(&hex).unwrap();
        let js_result = try_bundled_decrypt(&hex, &engine).await.unwrap();
        assert_eq!(rust_result, js_result, "Algo 7 (Weyl+MXS) parity failed");
    }

    #[tokio::test]
    async fn test_rust_js_parity_all_algos_bulk() {
        let engine = BoaJsEngine::new();
        let seeds: &[i32] = &[0, 1, -1, 42, 12345, i32::MAX, i32::MIN, 0x7F7F_7F7F];
        let plaintext = "https://cdn.example.com/path/to/video.mp4?token=abc123";

        for algo_id in 1..=7u8 {
            for &seed in seeds {
                let hex = encrypt_test_vector(algo_id, seed, plaintext);
                let rust_result = rust_decipher(&hex);
                let js_result = try_bundled_decrypt(&hex, &engine).await;
                assert_eq!(
                    rust_result, js_result,
                    "Parity failed for algo {algo_id}, seed {seed}"
                );
            }
        }
    }

    // =========================================================================
    // Boa initials extraction tests
    // =========================================================================

    #[tokio::test]
    async fn test_boa_initials_extraction() {
        let engine = BoaJsEngine::new();
        let html = r#"<script>window.initials = {"videoModel": {"title": "Test"}};</script>"#;
        let result = extract_initials_via_boa(html, &engine).await;
        assert!(result.is_some());
        let val = result.unwrap();
        assert_eq!(
            val.pointer("/videoModel/title").unwrap().as_str(),
            Some("Test")
        );
    }

    #[tokio::test]
    async fn test_boa_initials_extraction_minified() {
        let engine = BoaJsEngine::new();
        let html = r#"<script>window.initials={"videoModel":{"title":"Minified","sources":{"mp4":{"720p":"url"}}}};</script>"#;
        let result = extract_initials_via_boa(html, &engine).await;
        assert!(result.is_some());
    }

    #[tokio::test]
    async fn test_boa_initials_missing() {
        let engine = BoaJsEngine::new();
        let html = "<html><body>No initials</body></html>";
        let result = extract_initials_via_boa(html, &engine).await;
        assert!(result.is_none());
    }

    // =========================================================================
    // Bundled JS unit tests
    // =========================================================================

    #[tokio::test]
    async fn test_bundled_js_decipher_all_algos() {
        let engine = BoaJsEngine::new();
        for algo_id in 1..=7u8 {
            let hex = encrypt_test_vector(algo_id, 42, "hello");
            let result = try_bundled_decrypt(&hex, &engine).await;
            assert!(result.is_some(), "Algo {algo_id} should decipher");
            assert_eq!(result.unwrap(), "hello", "Algo {algo_id} wrong output");
        }
    }

    #[tokio::test]
    async fn test_bundled_js_unknown_algo() {
        let engine = BoaJsEngine::new();
        // Algo ID 0 — not valid, first byte is 00
        let result = try_bundled_decrypt("000000000041424344", &engine).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_bundled_js_short_hex() {
        let engine = BoaJsEngine::new();
        let result = try_bundled_decrypt("abcd", &engine).await;
        assert!(result.is_none());
    }

    // =========================================================================
    // Player JS discovery tests
    // =========================================================================

    #[test]
    fn test_find_player_script_url_standard() {
        let html =
            r#"<script src="https://cdn.xhamster.com/js/xplayer.bundle.abc123.js"></script>"#;
        let urls = find_player_script_urls(html);
        assert_eq!(urls.len(), 1);
        assert!(urls[0].contains("xplayer"));
    }

    #[test]
    fn test_find_player_script_url_cdn_versioned() {
        let html = r#"<script src="//static.xhcdn.com/xplayer/v4.2.1/player.min.js"></script>"#;
        let urls = find_player_script_urls(html);
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn test_find_player_script_url_none() {
        let html = r#"<script src="https://cdn.example.com/analytics.js"></script>"#;
        let urls = find_player_script_urls(html);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_find_player_script_url_multiple() {
        let html = r#"
            <script src="https://cdn.example.com/player-loader.js"></script>
            <script src="https://cdn.xhamster.com/js/xplayer.bundle.js"></script>
        "#;
        let urls = find_player_script_urls(html);
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn test_extract_decrypt_function_from_bundle() {
        let bundle = r#"
            function someHelper() { return 42; }
            function decipherUrl(hex) { var x = 1664525; /* ... */ }
            function anotherHelper() { return true; }
        "#;
        assert!(DECRYPT_FUNC_PATTERN.is_match(bundle));
        let func_name = DECRYPT_FUNC_PATTERN
            .captures(bundle)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str());
        assert_eq!(func_name, Some("decipherUrl"));
    }

    #[test]
    fn test_extract_decrypt_function_obfuscated() {
        // Minified with short variable names but same constants
        let bundle = "function a(b){var c=1664525;var d=b.charCodeAt(0);return c*d}";
        assert!(DECRYPT_FUNC_PATTERN.is_match(bundle));
    }

    // =========================================================================
    // End-to-end extraction tests
    // =========================================================================

    #[tokio::test]
    async fn test_full_extraction_modern_layout() {
        let engine = BoaJsEngine::new();
        // Build a realistic mock page with encrypted URLs.
        // Only HLS URLs are kept — standard direct URLs are CDN-blocked.
        let encrypted_hls = encrypt_test_vector(1, 100, "https://cdn.example.com/master.m3u8");
        // Standard URL that decrypts to an HLS-like path (media=hls4)
        let encrypted_std_hls =
            encrypt_test_vector(3, 200, "https://cdn.example.com/media=hls4/720.m3u8");

        let initials = serde_json::json!({
            "videoModel": {
                "title": "Test",
                "sources": {
                    "mp4": { "720p": "https://cdn.example.com/direct.mp4" }
                }
            },
            "xplayerSettings": {
                "sources": {
                    "hls": { "url": encrypted_hls },
                    "standard": {
                        "mp4": [{ "quality": "720", "url": encrypted_std_hls }]
                    }
                }
            }
        });

        let formats = super::super::formats::extract_from_initials(
            &initials,
            "https://xhamster.com/videos/test-123",
            &engine,
            None,
        )
        .await;

        assert!(!formats.is_empty(), "Should extract at least one format");
        // videoModel.sources direct URLs are CDN-blocked and skipped
        assert!(
            !formats.iter().any(|f| f.url.contains("direct.mp4")),
            "Direct videoModel URLs should be skipped (CDN-blocked)"
        );
        // HLS formats should be present
        assert!(
            formats.iter().any(|f| f.url.contains("master.m3u8")),
            "HLS master playlist should be present"
        );
    }

    #[tokio::test]
    async fn test_full_extraction_player_js_unavailable() {
        let engine = BoaJsEngine::new();
        // Use an HLS URL — standard direct URLs are CDN-blocked and skipped
        let encrypted =
            encrypt_test_vector(2, 42, "https://cdn.example.com/media=hls4/video.m3u8");

        let initials = serde_json::json!({
            "videoModel": {
                "sources": {}
            },
            "xplayerSettings": {
                "sources": {
                    "standard": {
                        "mp4": [{ "quality": "720", "url": encrypted }]
                    }
                }
            }
        });

        let formats = super::super::formats::extract_from_initials(
            &initials,
            "https://xhamster.com/videos/test-123",
            &engine,
            None,
        )
        .await;

        assert!(
            formats.iter().any(|f| f.url.contains("cdn.example.com")),
            "Bundled JS fallback should decrypt HLS URL successfully"
        );
    }

    #[tokio::test]
    async fn test_full_extraction_mixed_encrypted_plain() {
        let engine = BoaJsEngine::new();
        // HLS URL in standard section — only HLS URLs are kept
        let encrypted_hls =
            encrypt_test_vector(5, 999, "https://cdn.example.com/media=hls4/enc.m3u8");
        // Direct MP4 in standard section — should be skipped (CDN-blocked)
        let encrypted_mp4 = encrypt_test_vector(3, 100, "https://cdn.example.com/enc.mp4");

        let initials = serde_json::json!({
            "videoModel": {
                "sources": {
                    "mp4": { "1080p": "https://cdn.example.com/plain.mp4" }
                }
            },
            "xplayerSettings": {
                "sources": {
                    "standard": {
                        "h264": [{ "quality": "720", "url": encrypted_hls }],
                        "mp4": [{ "quality": "1080", "url": encrypted_mp4 }]
                    }
                }
            }
        });

        let formats = super::super::formats::extract_from_initials(
            &initials,
            "https://xhamster.com/videos/test-123",
            &engine,
            None,
        )
        .await;

        let urls: Vec<&str> = formats.iter().map(|f| f.url.as_str()).collect();
        // videoModel.sources direct URLs are CDN-blocked and skipped
        assert!(
            !urls.iter().any(|u| u.contains("plain.mp4")),
            "Direct videoModel URLs should be skipped (CDN-blocked)"
        );
        // Standard direct MP4 URLs are also CDN-blocked and skipped
        assert!(
            !urls.iter().any(|u| u.contains("enc.mp4")),
            "Standard direct MP4 URLs should be skipped (CDN-blocked)"
        );
        // HLS URLs from standard section should be kept
        assert!(
            urls.iter().any(|u| u.contains("enc.m3u8")),
            "HLS URL from standard section should be present"
        );
    }

    // =========================================================================
    // Edge case and adversarial tests
    // =========================================================================

    #[tokio::test]
    async fn test_boa_malformed_js() {
        let engine = BoaJsEngine::new();
        let html = "<script>window.initials = {invalid json here</script>";
        let result = extract_initials_via_boa(html, &engine).await;
        assert!(result.is_none(), "Malformed JS should return None");
    }

    #[tokio::test]
    async fn test_decipher_unicode_url() {
        let engine = BoaJsEngine::new();
        // URL with non-ASCII characters after decryption
        let hex = encrypt_test_vector(1, 42, "https://example.com/vid\u{00e9}o.mp4");
        let result = try_bundled_decrypt(&hex, &engine).await;
        assert!(result.is_some());
        assert!(result.unwrap().contains("vid"));
    }

    #[tokio::test]
    async fn test_decipher_partial_hex() {
        let engine = BoaJsEngine::new();
        // Odd-length hex — invalid (11 chars)
        let result = try_bundled_decrypt("01000000004142430", &engine).await;
        // May succeed (valid algo+seed+3 bytes) or fail — just shouldn't panic
        let _ = result;
    }

    #[tokio::test]
    async fn test_decipher_empty_string() {
        let engine = BoaJsEngine::new();
        let result = try_bundled_decrypt("", &engine).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_decipher_url_with_hex_segment() {
        let engine = BoaJsEngine::new();
        // Encrypt a path segment, then embed it in a full URL with remainder.
        // This tests the URL-with-hex-segment path in decipherFormatUrl.
        let plaintext = "decrypted-path-segment";
        let hex_segment = encrypt_test_vector(1, 42, plaintext);
        let url_with_hex = format!("https://cdn.example.com/{hex_segment}/rest/of/path.m3u8");

        let result = try_bundled_decrypt(&url_with_hex, &engine).await;
        assert!(result.is_some(), "Should handle URL-with-hex-segment");
        let decrypted = result.unwrap();
        assert!(
            decrypted.contains(plaintext),
            "Decrypted URL should contain plaintext segment: {decrypted}"
        );
        assert!(
            decrypted.contains("/rest/of/path.m3u8"),
            "Remainder should be preserved: {decrypted}"
        );
        assert!(
            decrypted.starts_with("https://cdn.example.com/"),
            "Host should be preserved: {decrypted}"
        );
    }

    #[tokio::test]
    async fn test_decipher_url_with_short_hex_not_matched() {
        let engine = BoaJsEngine::new();
        // Hex segment shorter than 12 chars — should not match the URL pattern
        let url = "https://cdn.example.com/abcd1234/path.m3u8";
        let result = try_bundled_decrypt(url, &engine).await;
        assert!(result.is_none(), "Short hex in URL should not be deciphered");
    }

    #[tokio::test]
    async fn test_boa_very_large_initials() {
        let engine = BoaJsEngine::new();
        // Generate a large but valid initials object
        let large_value = "x".repeat(100_000);
        let html = format!(
            r#"<script>window.initials = {{"videoModel": {{"title": "{large_value}"}}}};</script>"#
        );
        let result = extract_initials_via_boa(&html, &engine).await;
        assert!(result.is_some(), "Should handle large scripts");
    }
}
