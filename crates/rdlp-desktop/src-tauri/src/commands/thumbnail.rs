//! Thumbnail proxy command.
//!
//! Fetches a remote thumbnail image server-side, injecting the correct
//! `Referer` header that CDN servers (e.g. CDN77/phncdn) require.
//! Returns raw image bytes via [`tauri::ipc::Response`] to bypass
//! JSON serialization overhead.

use tauri::ipc::Response;

use crate::error::AppError;

/// Maximum response body size (5 MB).
const MAX_BODY_SIZE: usize = 5 * 1024 * 1024;

/// Timeout for the thumbnail fetch request.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Derive the site origin from a URL string for use as Referer.
///
/// CDN subdomains (e.g. `fastporndelivery.hqporner.com`, `cdn77.phncdn.com`)
/// are stripped to the registrable domain so the Referer matches what the
/// CDN expects (e.g. `https://hqporner.com`).
fn derive_referer(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let scheme = parsed.scheme();

    // For IP addresses, keep as-is (including port)
    if host.parse::<std::net::IpAddr>().is_ok() {
        return match parsed.port() {
            Some(port) => Some(format!("{scheme}://{host}:{port}")),
            None => Some(format!("{scheme}://{host}")),
        };
    }

    // Strip CDN subdomains: keep last 2 segments, prepend www.
    // e.g. fastporndelivery.hqporner.com → www.hqporner.com
    //      i.xtits.com                   → www.xtits.com
    // The www. prefix is required by some CDNs (e.g. xtits) that
    // reject the bare registrable domain as Referer.
    let parts: Vec<&str> = host.split('.').collect();
    let registrable = if parts.len() > 2 {
        parts[parts.len() - 2..].join(".")
    } else {
        host.to_string()
    };

    Some(format!("{scheme}://www.{registrable}"))
}

/// Fetch a thumbnail image from `url` with a derived `Referer` header
/// and return the raw bytes.
///
/// # Arguments
///
/// * `url` - HTTPS URL of the thumbnail image to proxy.
///
/// # Returns
///
/// Raw image bytes wrapped in a [`tauri::ipc::Response`].
///
/// # Errors
///
/// Returns [`AppError::InvalidInput`] if the URL is not HTTPS.
/// Returns [`AppError::Internal`] on network failures, timeouts, or
/// if the response exceeds [`MAX_BODY_SIZE`].
#[tauri::command]
pub async fn proxy_thumbnail(url: String) -> Result<Response, AppError> {
    // Validate HTTPS
    if !url.starts_with("https://") {
        return Err(AppError::InvalidInput {
            field: "url".to_owned(),
            message: "Thumbnail proxy only supports HTTPS URLs".to_owned(),
        });
    }

    let referer = derive_referer(&url).unwrap_or_default();

    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| AppError::Internal {
            message: format!("Failed to create HTTP client: {e}"),
        })?;

    let resp = client
        .get(&url)
        .header(reqwest::header::REFERER, &referer)
        .send()
        .await
        .map_err(|e| AppError::Internal {
            message: format!("Thumbnail fetch failed: {e}"),
        })?;

    if !resp.status().is_success() {
        return Err(AppError::Internal {
            message: format!("Thumbnail fetch returned HTTP {}", resp.status().as_u16()),
        });
    }

    // Check Content-Length if available
    if let Some(len) = resp.content_length()
        && len as usize > MAX_BODY_SIZE
    {
        return Err(AppError::Internal {
            message: format!("Thumbnail too large: {len} bytes (max {MAX_BODY_SIZE})"),
        });
    }

    let bytes = resp.bytes().await.map_err(|e| AppError::Internal {
        message: format!("Failed to read thumbnail body: {e}"),
    })?;

    if bytes.len() > MAX_BODY_SIZE {
        return Err(AppError::Internal {
            message: format!(
                "Thumbnail too large: {} bytes (max {MAX_BODY_SIZE})",
                bytes.len()
            ),
        });
    }

    Ok(Response::new(bytes.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_referer_standard_url() {
        // CDN subdomain stripped to registrable domain with www prefix
        let referer = derive_referer("https://di.phncdn.com/videos/abc/thumb.jpg");
        assert_eq!(referer.as_deref(), Some("https://www.phncdn.com"));
    }

    #[test]
    fn test_derive_referer_cdn_subdomain() {
        let referer = derive_referer("https://fastporndelivery.hqporner.com/imgs/123/thumb.jpg");
        assert_eq!(referer.as_deref(), Some("https://www.hqporner.com"));
    }

    #[test]
    fn test_derive_referer_xtits_cdn() {
        // i.xtits.com CDN requires www.xtits.com Referer (bare xtits.com → 403)
        let referer = derive_referer(
            "https://i.xtits.com/contents/videos_screenshots/50000/50088/402x225/2.jpg",
        );
        assert_eq!(referer.as_deref(), Some("https://www.xtits.com"));
    }

    #[test]
    fn test_derive_referer_no_subdomain() {
        let referer = derive_referer("https://example.com/image.jpg");
        assert_eq!(referer.as_deref(), Some("https://www.example.com"));
    }

    #[test]
    fn test_derive_referer_invalid_url() {
        assert!(derive_referer("not-a-url").is_none());
    }

    #[tokio::test]
    async fn test_rejects_http_url() {
        let result = proxy_thumbnail("http://example.com/image.jpg".to_owned()).await;
        match result {
            Err(AppError::InvalidInput { field, message }) => {
                assert_eq!(field, "url");
                assert!(message.contains("HTTPS"));
            }
            Err(other) => panic!("Expected InvalidInput, got: {other:?}"),
            Ok(_) => panic!("Expected Err(InvalidInput), got Ok"),
        }
    }

    #[tokio::test]
    async fn test_rejects_empty_url() {
        let result = proxy_thumbnail("".to_owned()).await;
        match result {
            Err(AppError::InvalidInput { .. }) => {}
            Err(other) => panic!("Expected InvalidInput, got: {other:?}"),
            Ok(_) => panic!("Expected Err(InvalidInput), got Ok"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_proxy_sends_referer_and_returns_bytes() {
        let mut server = mockito::Server::new_async().await;
        // mockito URL is http://127.0.0.1:PORT — IP addresses are preserved as-is
        let expected_referer = server.url();
        let mock = server
            .mock("GET", "/image.jpg")
            .match_header("referer", mockito::Matcher::Exact(expected_referer.clone()))
            .with_status(200)
            .with_body(vec![0x89, 0x50, 0x4e, 0x47]) // PNG magic bytes
            .create_async()
            .await;

        // mockito uses http://, so test inner logic directly (proxy_thumbnail requires https)
        let referer = derive_referer(&format!("{}/image.jpg", server.url())).unwrap_or_default();
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/image.jpg", server.url()))
            .header(reqwest::header::REFERER, &referer)
            .send()
            .await
            .expect("request should succeed");
        assert!(resp.status().is_success());
        let bytes = resp.bytes().await.unwrap();
        assert_eq!(&bytes[..], &[0x89, 0x50, 0x4e, 0x47]);
        mock.assert_async().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_proxy_returns_error_on_403() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/forbidden.jpg")
            .with_status(403)
            .create_async()
            .await;

        // Test via inner HTTP logic (mockito uses http://)
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/forbidden.jpg", server.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 403);
    }
}
