//! Direct media URL detection via prefetch.
//!
//! Before fetching full HTML, we send a range request for the first 512 bytes
//! and check the Content-Type header. This detects direct media links and
//! HLS manifests without parsing HTML.

use anyhow::Context;
use rdlp_core::ExtractionContext;
use url::Url;

use super::patterns;

/// Maximum bytes to prefetch for content sniffing.
const PREFETCH_SIZE: usize = 512;

/// Response from a 512-byte prefetch request.
pub(crate) struct PrefetchResponse {
    /// Content-Type header value
    pub content_type: Option<String>,
    /// First 512 bytes of the response body
    pub bytes: Vec<u8>,
    /// Content-Length from headers, if present
    pub content_length: Option<u64>,
}

impl PrefetchResponse {
    /// Check if Content-Type indicates direct media.
    pub fn is_media_content_type(&self) -> bool {
        self.content_type
            .as_ref()
            .is_some_and(|ct| patterns::is_media_content_type(ct))
    }

    /// Check if Content-Type indicates a DASH manifest.
    pub fn is_dash_content_type(&self) -> bool {
        self.content_type
            .as_ref()
            .is_some_and(|ct| ct.to_lowercase().contains("dash+xml"))
    }

    /// Check if Content-Type indicates HTML.
    pub fn is_html_content_type(&self) -> bool {
        self.content_type
            .as_ref()
            .is_some_and(|ct| patterns::is_html_content_type(ct))
    }

    /// Check if prefetched bytes start with HLS magic (`#EXTM3U`).
    pub fn is_hls_manifest(&self) -> bool {
        self.bytes.starts_with(b"#EXTM3U")
    }

    /// Check if prefetched bytes look like an MPD manifest.
    pub fn is_mpd_manifest(&self) -> bool {
        let text = String::from_utf8_lossy(&self.bytes);
        let trimmed = text.trim_start();
        trimmed.starts_with("<?xml") && trimmed.contains("<MPD") || trimmed.starts_with("<MPD")
    }
}

/// Perform a 512-byte prefetch to detect direct media URLs.
pub(crate) async fn prefetch(
    url: &str,
    ctx: &ExtractionContext,
) -> anyhow::Result<PrefetchResponse> {
    let response = ctx
        .http_client
        .get(url)
        .header("Accept-Encoding", "identity")
        .header("Range", format!("bytes=0-{}", PREFETCH_SIZE - 1))
        .send()
        .await
        .context("prefetch request failed")?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let content_length = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let bytes = response
        .bytes()
        .await
        .context("failed to read prefetch bytes")?
        .to_vec();

    Ok(PrefetchResponse {
        content_type,
        bytes,
        content_length,
    })
}

/// Extract a title from a direct media URL (filename without extension).
pub(crate) fn title_from_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| {
            let path = u.path();
            let filename = path.rsplit('/').next()?;
            let name = filename.split('.').next()?;
            let decoded = urlencoding::decode(name).ok()?;
            let name = decoded.trim();
            if name.is_empty() {
                return None;
            }
            Some(name.to_string())
        })
        .unwrap_or_else(|| "Untitled".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_url_extracts_filename() {
        assert_eq!(
            title_from_url("https://cdn.example.com/My%20Video.mp4"),
            "My Video"
        );
        assert_eq!(title_from_url("https://cdn.example.com/video.mp4"), "video");
    }

    #[test]
    fn title_from_url_fallback() {
        assert_eq!(title_from_url("https://example.com/"), "Untitled");
        assert_eq!(title_from_url("not-a-url"), "Untitled");
    }

    #[test]
    fn prefetch_response_hls_detection() {
        let resp = PrefetchResponse {
            content_type: Some("application/vnd.apple.mpegurl".to_string()),
            bytes: b"#EXTM3U\n#EXT-X-VERSION:3".to_vec(),
            content_length: None,
        };
        assert!(resp.is_hls_manifest());
        assert!(resp.is_media_content_type());
        assert!(!resp.is_html_content_type());
    }

    #[test]
    fn prefetch_response_html_detection() {
        let resp = PrefetchResponse {
            content_type: Some("text/html; charset=utf-8".to_string()),
            bytes: b"<!DOCTYPE html><html>".to_vec(),
            content_length: None,
        };
        assert!(!resp.is_hls_manifest());
        assert!(!resp.is_media_content_type());
        assert!(resp.is_html_content_type());
    }

    #[test]
    fn prefetch_response_mpd_detection() {
        let resp = PrefetchResponse {
            content_type: Some("application/dash+xml".to_string()),
            bytes: b"<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema\">".to_vec(),
            content_length: None,
        };
        assert!(resp.is_mpd_manifest());
        assert!(resp.is_media_content_type());
    }
}
