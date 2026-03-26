//! AJAX and XML parsing for TNAFlix network sites
//!
//! Handles site-specific data fetching:
//! - EMPFlix: AJAX endpoint for video player HTML
//! - MovieFap: XML endpoint for video sources

use rdlp_core::{ExtractionContext, RdlpError, Result, check_http_response};
use scraper::Html;

use crate::base::common::BaseExtractor;
use crate::base::tnaflix_network::{TnaFlixNetworkBase, VideoMetadata};

/// Parse EMPFlix AJAX JSON response to extract video sources
///
/// EMPFlix uses an AJAX endpoint that returns JSON with embedded HTML
/// containing the video player with `<source>` tags.
pub async fn parse_empflix_ajax(
    base: &TnaFlixNetworkBase,
    video_id: &str,
    referer: &str,
    ctx: &ExtractionContext,
) -> Result<Vec<VideoMetadata>> {
    // Fetch JSON from AJAX endpoint
    let ajax_url = format!("https://www.empflix.com/ajax/video-player/{video_id}");

    let response = ctx
        .http_client
        .get(&ajax_url)
        .header("Referer", referer)
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch EMPFlix AJAX: {e}")))?;

    check_http_response(&response)?;

    let json_text = response
        .text()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to read AJAX response: {e}")))?;

    BaseExtractor::log_content_if_verbose(ctx, "EMPFlix", "AJAX Response", &json_text, 500);

    // Parse JSON to extract HTML field
    let json: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|e| RdlpError::Extraction(format!("Failed to parse AJAX JSON: {e}")))?;

    let html_str = json
        .get("html")
        .and_then(|h| h.as_str())
        .ok_or_else(|| RdlpError::Extraction("No 'html' field in AJAX response".to_string()))?;

    // Parse the HTML to extract <source> tags using base
    let html = Html::parse_document(html_str);
    let video_data = base.parse_video_sources(&html);

    if video_data.is_empty() {
        return Err(RdlpError::Extraction(format!(
            "No video sources found in EMPFlix AJAX HTML. URL: {referer}"
        )));
    }

    Ok(video_data)
}

/// Parse MovieFap XML response to extract video sources
///
/// MovieFap uses a cdn.php endpoint that returns XML with video quality options.
pub async fn parse_moviefap_xml(
    base: &TnaFlixNetworkBase,
    cdn_url: &str,
    referer: &str,
    ctx: &ExtractionContext,
) -> Result<Vec<VideoMetadata>> {
    // Fetch the XML from cdn.php (matching browser headers)
    let response = ctx
        .http_client
        .get(cdn_url)
        .header("Referer", referer)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch MovieFap XML: {e}")))?;

    check_http_response(&response)?;

    let xml_text = response
        .text()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to read XML response: {e}")))?;

    BaseExtractor::log_content_if_verbose(ctx, "MovieFap", "XML Response", &xml_text, 1000);

    // Use base to parse XML
    let video_data = base.parse_moviefap_xml(&xml_text);

    if video_data.is_empty() {
        return Err(RdlpError::Extraction(format!(
            "No video sources found in MovieFap XML response from: {cdn_url}"
        )));
    }

    Ok(video_data)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_ajax_url_format() {
        let video_id = "123456";
        let ajax_url = format!("https://www.empflix.com/ajax/video-player/{video_id}");
        assert_eq!(ajax_url, "https://www.empflix.com/ajax/video-player/123456");
    }
}
