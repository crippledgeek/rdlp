//! Shared `expand_hls_in_place` post-processing helper.
//!
//! Hoisted from the spankbang extractor in PR #258 so every HLS-emitting
//! extractor can call the same code path. Replaces every `Format` row whose
//! protocol is `M3u8` or `M3u8Native` with the per-variant rows produced by
//! [`expand_hls_url`]. On any [`HlsExpandError`] the row is dropped entirely
//! (no fallback to legacy variant-URL path).
//!
//! ## Convention
//!
//! Size probes (`detect_format_sizes_lazy`, custom HEAD probes,
//! segment-count enrichment) MUST run AFTER `expand_hls_in_place`,
//! not before. Probing the master-playlist URL yields manifest size,
//! not per-variant size; per-variant size estimation requires the
//! `Fragment` list this helper produces.
//!
//! Reference call shape (xhamster, the model):
//! ```ignore
//! let formats = expand_hls_in_place(formats, http_client).await;
//! let (formats, flags) = detect_format_sizes_lazy(formats, ctx, name).await;
//! ```
//!
//! See issue #269 for the audit and remediation history.

use std::sync::Arc;

use rdlp_types::{DownloadProtocol, Format};

use super::expand::expand_hls_url;

/// Replace every M3u8 / M3u8Native row in `formats` with its per-variant
/// expansion. Non-HLS rows pass through unchanged. Expand failures drop the
/// row entirely (no fallback to legacy variant-URL path).
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
                        "HLS expand failed for {} ({e}) — dropping format row",
                        rdlp_security::sanitize_for_logging(&f.url)
                    );
                    // Format row is dropped. If all formats fail, the resulting
                    // Vec is empty and the calling extractor surfaces the
                    // existing 'No formats found' error path.
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
    async fn drops_format_on_encrypted() {
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
        assert!(
            out.is_empty(),
            "encrypted format must be dropped (no legacy fallback)"
        );
    }

    #[tokio::test]
    async fn drops_format_on_live() {
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
        assert!(
            out.is_empty(),
            "live format must be dropped (no legacy fallback)"
        );
    }

    /// Contract-shape test for the HLS extractor helper pair.
    ///
    /// Documents the expected behaviour: `expand_hls_in_place` then
    /// `detect_format_sizes_lazy` produces per-variant Format rows with
    /// `fragments.is_some()`, and the master m3u8 is fetched exactly once.
    ///
    /// NOTE: this test does NOT regress against order reversal — both helpers
    /// independently fetch the master once and produce equivalent format rows.
    /// The actual order regression guard for issue #279 is
    /// [`test_extractor_call_order_expand_before_detect`] below, which checks
    /// the textual order of the helper calls in each extractor's source.
    #[tokio::test]
    async fn test_helper_pair_contract_shape() {
        use crate::hls::test_support::{MASTER_TWO_VARIANTS, VARIANT_MEDIA, test_ctx};
        use rdlp_types::{DownloadProtocol, Format};
        use std::sync::Arc;

        let mut server = mockito::Server::new_async().await;

        let master = server
            .mock("GET", "/master.m3u8")
            .with_body(MASTER_TWO_VARIANTS)
            .expect(1)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v720.m3u8")
            .with_body(VARIANT_MEDIA)
            .expect_at_least(1)
            .create_async()
            .await;
        let _v360 = server
            .mock("GET", "/v360.m3u8")
            .with_body(VARIANT_MEDIA)
            .expect_at_least(1)
            .create_async()
            .await;

        let master_url = format!("{}/master.m3u8", server.url());
        let f = Format::new("hls", &master_url, "m3u8", DownloadProtocol::M3u8);

        let ctx = test_ctx();
        let http: Arc<wreq::Client> = ctx.http_client.clone();

        // Production order. Reverting these two lines makes master.assert fail.
        let formats = expand_hls_in_place(vec![f], http).await;
        let (formats, _flags) = crate::hls::detect_format_sizes_lazy(formats, &ctx, "test").await;

        assert!(
            !formats.is_empty(),
            "expand must produce at least one expanded format row"
        );
        assert!(
            formats.iter().all(|fmt| fmt.fragments.is_some()),
            "every expanded format must carry pre-resolved fragments"
        );
        master.assert_async().await;
    }

    /// Static regression guard for issue #279.
    ///
    /// Asserts that the `expand_hls_in_place(` call appears textually BEFORE
    /// the `detect_format_sizes_lazy(` call in each affected extractor's
    /// source. A future refactor that reverts the order trips this test.
    ///
    /// Sources are pinned via `include_str!` so cargo recompiles this test
    /// whenever the extractor files change — no runtime `fs` access (the
    /// workspace bans `std::fs::read_to_string` outside known seams).
    ///
    /// Why a textual check: both helpers independently fetch the master m3u8
    /// once and produce equivalent format rows, so a runtime mockito-based
    /// observable cannot distinguish the orderings end-to-end. The textual
    /// check is the cheapest deterministic guard available.
    #[test]
    fn test_extractor_call_order_expand_before_detect() {
        let cases: &[(&str, &str)] = &[
            (
                "extractors/koreanpornmovie/mod.rs",
                include_str!("../extractors/koreanpornmovie/mod.rs"),
            ),
            (
                "extractors/nine_anime/mod.rs",
                include_str!("../extractors/nine_anime/mod.rs"),
            ),
            (
                "extractors/hqporner/mod.rs",
                include_str!("../extractors/hqporner/mod.rs"),
            ),
            (
                "extractors/pornhub/mod.rs",
                include_str!("../extractors/pornhub/mod.rs"),
            ),
            (
                "extractors/redtube/mod.rs",
                include_str!("../extractors/redtube/mod.rs"),
            ),
            (
                "extractors/xtits/mod.rs",
                include_str!("../extractors/xtits/mod.rs"),
            ),
        ];

        for (label, src) in cases {
            // Production code always precedes any test module, so the FIRST
            // occurrence of each helper is the production call. We deliberately
            // do NOT split on `#[cfg(test)]`: some extractors declare
            // `#[cfg(test)] mod tests;` (an external test file) near the TOP of
            // the module, which would wrongly truncate the production body.
            // First-occurrence is robust to test-module placement and still
            // fails correctly if production omits a call (the only match would
            // then be in test code, after the other call, or absent entirely).
            let expand_pos = src
                .find("expand_hls_in_place(")
                .unwrap_or_else(|| panic!("`expand_hls_in_place(` call not found in {label}"));
            let detect_pos = src
                .find("detect_format_sizes_lazy(")
                .unwrap_or_else(|| panic!("`detect_format_sizes_lazy(` call not found in {label}"));

            assert!(
                expand_pos < detect_pos,
                "{label}: `expand_hls_in_place(` must appear before \
                 `detect_format_sizes_lazy(` (issue #269 / #279). \
                 Found expand at byte {expand_pos}, detect at byte {detect_pos}."
            );
        }
    }
}
