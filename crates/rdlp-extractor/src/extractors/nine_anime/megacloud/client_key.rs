//! Client key extraction from Megacloud v3 embed pages.
//!
//! The key is hidden using one of several obfuscation methods:
//! 1. Meta tag: `<meta name="_gg_fb" content="KEY">`
//! 2. Comment: `<!-- _is_th:KEY -->`
//! 3. 3-part script: `window._lk_db = {x:"P1", y:"P2", z:"P3"}`
//! 4. Div data attribute: `<div data-dpi="KEY" ...></div>`
//! 5. Script nonce: `<script nonce="KEY">`
//! 6. Window variable: `window._xy_ws = "KEY"`

use anyhow::Context as _;
use lazy_regex::{Lazy, Regex, lazy_regex};
use rdlp_core::{ExtractionContext, RdlpError, Result};

use super::MEGACLOUD_API;

/// Regex patterns for extracting the client key from the embed page.
static META_PATTERN: Lazy<Regex> = lazy_regex!(r#"<meta name="_gg_fb" content="[a-zA-Z0-9]+">"#);
static COMMENT_PATTERN: Lazy<Regex> = lazy_regex!(r#"<!--\s+_is_th:[0-9a-zA-Z]+\s+-->"#);
static LK_DB_PATTERN: Lazy<Regex> = lazy_regex!(
    r#"<script>window\._lk_db\s+=\s+\{[xyz]:\s+["'][a-zA-Z0-9]+["'],\s+[xyz]:\s+["'][a-zA-Z0-9]+["'],\s+[xyz]:\s+["'][a-zA-Z0-9]+["']\};</script>"#
);
static DIV_PATTERN: Lazy<Regex> = lazy_regex!(r#"<div\s+data-dpi="[0-9a-zA-Z]+"\s+[^>]*></div>"#);
static NONCE_PATTERN: Lazy<Regex> = lazy_regex!(r#"<script nonce="[0-9a-zA-Z]+">"#);
static WINDOW_VAR_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"<script>window\._xy_ws = ['"`][0-9a-zA-Z]+['"`];</script>"#);

/// General quoted-key pattern for most obfuscation methods.
static QUOTED_KEY: Lazy<Regex> = lazy_regex!(r#""[a-zA-Z0-9]+""#);

/// Pattern for the 3-part `_lk_db` key components.
static LK_DB_X: Lazy<Regex> = lazy_regex!(r#"x:\s+"[a-zA-Z0-9]+""#);
static LK_DB_Y: Lazy<Regex> = lazy_regex!(r#"y:\s+"[a-zA-Z0-9]+""#);
static LK_DB_Z: Lazy<Regex> = lazy_regex!(r#"z:\s+"[a-zA-Z0-9]+""#);

/// Comment key pattern (no quotes, colon-delimited).
static COMMENT_KEY: Lazy<Regex> = lazy_regex!(r":[a-zA-Z0-9]+ ");

/// Fetch the v3 embed page and extract the client key.
pub(super) async fn extract_client_key(source_id: &str, ctx: &ExtractionContext) -> Result<String> {
    extract_client_key_impl(source_id, ctx)
        .await
        .map_err(|e| RdlpError::Extraction {
            message: format!("{e:#}"),
            url: None,
        })
}

async fn extract_client_key_impl(
    source_id: &str,
    ctx: &ExtractionContext,
) -> anyhow::Result<String> {
    let url = format!("{MEGACLOUD_API}/embed-2/v3/e-1/{source_id}");
    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", "https://9animetv.to/")
        .send()
        .await
        .with_context(|| {
            format!("failed to fetch megacloud v3 embed page for source_id={source_id}")
        })?;

    let html = response.text().await.with_context(|| {
        format!("failed to read megacloud v3 embed body for source_id={source_id}")
    })?;

    parse_client_key(&html)
}

/// Parse the client key from embed page HTML using pattern matching.
pub(super) fn parse_client_key(html: &str) -> anyhow::Result<String> {
    // Try each pattern in order
    let patterns = [
        (0, &*META_PATTERN),
        (1, &*COMMENT_PATTERN),
        (2, &*LK_DB_PATTERN),
        (3, &*DIV_PATTERN),
        (4, &*NONCE_PATTERN),
        (5, &*WINDOW_VAR_PATTERN),
    ];

    let (pattern_idx, text) = patterns
        .iter()
        .find_map(|(idx, pattern)| pattern.find(html).map(|m| (*idx, m.as_str().to_string())))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not find megacloud client key in embed page (no pattern matched)"
            )
        })?;

    match pattern_idx {
        // Pattern 1: Comment -- key follows colon, no quotes
        1 => {
            let key_match = COMMENT_KEY.find(&text).ok_or_else(|| {
                anyhow::anyhow!("failed to extract megacloud client key from comment pattern")
            })?;
            Ok(key_match.as_str().replace([':', ' '], ""))
        }
        // Pattern 2: 3-part _lk_db script -- assemble x+y+z
        2 => {
            let part_regexes = [&*LK_DB_X, &*LK_DB_Y, &*LK_DB_Z];
            let mut parts = Vec::new();
            for part_re in part_regexes.iter() {
                let part_match = part_re.find(&text).ok_or_else(|| {
                    anyhow::anyhow!("failed to extract _lk_db key part from megacloud embed")
                })?;
                let val = QUOTED_KEY.find(part_match.as_str()).ok_or_else(|| {
                    anyhow::anyhow!("failed to extract quoted value from _lk_db megacloud key")
                })?;
                parts.push(val.as_str().replace('"', ""));
            }
            Ok(parts.join(""))
        }
        // All other patterns -- extract quoted key
        _ => {
            let key_match = QUOTED_KEY.find(&text)
                .ok_or_else(|| anyhow::anyhow!("failed to extract quoted megacloud client key from matched pattern {pattern_idx}"))?;
            Ok(key_match.as_str().replace('"', ""))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_client_key_meta_tag() {
        let html = r#"<html><head><meta name="_gg_fb" content="abc123XYZ"></head></html>"#;
        let key = parse_client_key(html).unwrap();
        assert_eq!(key, "abc123XYZ");
    }

    #[test]
    fn test_parse_client_key_comment() {
        let html = r#"<html><!-- _is_th:secretKey42 --></html>"#;
        let key = parse_client_key(html).unwrap();
        assert_eq!(key, "secretKey42");
    }

    #[test]
    fn test_parse_client_key_div() {
        let html = r#"<html><div data-dpi="myKey99" style="display:none"></div></html>"#;
        let key = parse_client_key(html).unwrap();
        assert_eq!(key, "myKey99");
    }

    #[test]
    fn test_parse_client_key_nonce() {
        let html = r#"<html><script nonce="nonceKey77">var x=1;</script></html>"#;
        let key = parse_client_key(html).unwrap();
        assert_eq!(key, "nonceKey77");
    }

    #[test]
    fn test_parse_client_key_window_var() {
        let html = r#"<html><script>window._xy_ws = "windowKey55";</script></html>"#;
        let key = parse_client_key(html).unwrap();
        assert_eq!(key, "windowKey55");
    }

    #[test]
    fn test_parse_client_key_lk_db() {
        let html = r#"<html><script>window._lk_db = {x: "part1", y: "part2", z: "part3"};</script></html>"#;
        let key = parse_client_key(html).unwrap();
        assert_eq!(key, "part1part2part3");
    }

    #[test]
    fn test_parse_client_key_no_match() {
        let html = r#"<html><head><title>Test</title></head></html>"#;
        assert!(parse_client_key(html).is_err());
    }
}
