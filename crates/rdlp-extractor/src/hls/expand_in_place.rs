//! Shared `expand_hls_in_place` post-processing helper.
//!
//! Hoisted from the spankbang extractor in PR #258 so every HLS-emitting
//! extractor can call the same code path. Replaces every `Format` row whose
//! protocol is `M3u8` or `M3u8Native` with the per-variant rows produced by
//! [`expand_hls_url`]. On any [`HlsExpandError`] the original row is kept so
//! the legacy variant-URL download path still handles risky playlists
//! (encrypted, live, byte-range init, multi-init, etc.).

use std::sync::Arc;

use rdlp_types::{DownloadProtocol, Format};

use super::expand::expand_hls_url;

/// Replace every M3u8 / M3u8Native row in `formats` with its per-variant
/// expansion. Non-HLS rows pass through unchanged. Expand failures keep the
/// original row (graceful fallback to the legacy variant-URL path).
pub async fn expand_hls_in_place(formats: Vec<Format>, http: Arc<wreq::Client>) -> Vec<Format> {
    let mut expanded = Vec::with_capacity(formats.len());
    for f in formats {
        if matches!(
            f.protocol,
            DownloadProtocol::M3u8 | DownloadProtocol::M3u8Native
        ) {
            match expand_hls_url(&f, Arc::clone(&http)).await {
                Ok(rows) => expanded.extend(rows),
                Err(e) => {
                    log::warn!(
                        "HLS expand failed for {} ({e}) — falling back to legacy variant-URL path",
                        rdlp_security::sanitize_for_logging(&f.url)
                    );
                    expanded.push(f);
                }
            }
        } else {
            expanded.push(f);
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replaces_m3u8_rows_with_fragments() {
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/v.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXTINF:6.0,\nseg-2.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let url = format!("{}/v.m3u8", server.url());
        let f = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8);

        let http = Arc::new(wreq::Client::new());
        let out = expand_hls_in_place(vec![f], http).await;
        assert_eq!(out.len(), 1);
        assert!(out[0].fragments.is_some());
        assert_eq!(out[0].fragments.as_ref().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn replaces_m3u8_native_rows_with_fragments() {
        // Regression guard: pre-hoist version only matched DownloadProtocol::M3u8
        // and silently passed M3u8Native rows through without expansion. Eporner
        // and several other extractors emit M3u8Native, not M3u8.
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/native.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXTINF:6.0,\nseg-2.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let url = format!("{}/native.m3u8", server.url());
        let f = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8Native);

        let http = Arc::new(wreq::Client::new());
        let out = expand_hls_in_place(vec![f], http).await;
        assert_eq!(out.len(), 1, "M3u8Native must expand identically to M3u8");
        assert!(out[0].fragments.is_some());
        assert_eq!(out[0].fragments.as_ref().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn preserves_non_hls_rows() {
        let mp4 = Format::new(
            "1080p",
            "https://h.com/x.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        let http = Arc::new(wreq::Client::new());
        let out = expand_hls_in_place(vec![mp4.clone()], http).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].format_id, "1080p");
        assert!(out[0].fragments.is_none(), "MP4 row untouched");
    }

    #[tokio::test]
    async fn keeps_original_on_encrypted() {
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/enc.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXT-X-KEY:METHOD=AES-128,URI=\"https://h.com/key\"\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let url = format!("{}/enc.m3u8", server.url());
        let f = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8);

        let http = Arc::new(wreq::Client::new());
        let out = expand_hls_in_place(vec![f.clone()], http).await;
        assert_eq!(out.len(), 1, "graceful fallback keeps original");
        assert_eq!(out[0].url, f.url);
        assert!(out[0].fragments.is_none(), "no fragments on fallback");
    }

    #[tokio::test]
    async fn keeps_original_on_live() {
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/live.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n", // no #EXT-X-ENDLIST
            )
            .create_async()
            .await;

        let url = format!("{}/live.m3u8", server.url());
        let f = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8);

        let http = Arc::new(wreq::Client::new());
        let out = expand_hls_in_place(vec![f.clone()], http).await;
        assert_eq!(out.len(), 1);
        assert!(out[0].fragments.is_none());
    }
}
