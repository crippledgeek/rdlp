//! Shared `expand_hls_in_place` post-processing helper.
//!
//! Hoisted from the spankbang extractor in PR #258 so every HLS-emitting
//! extractor can call the same code path. Replaces every `Format` row whose
//! protocol is `M3u8` or `M3u8Native` with the per-variant rows produced by
//! [`expand_hls_url`]. On any [`HlsExpandError`] the row is dropped entirely
//! (no fallback to legacy variant-URL path) — including a `seed.url` refused
//! by `expand_hls_url`'s security gate, so one hostile entry in a ladder costs
//! only its own rendition (issue #660).
//!
//! ## Convention
//!
//! Size probes (`detect_format_sizes_lazy`, custom HEAD probes,
//! segment-count enrichment) MUST run AFTER `expand_hls_in_place`,
//! not before. Probing the master-playlist URL yields manifest size,
//! not per-variant size; per-variant size estimation requires the
//! `Fragment` list this helper produces.
//!
//! Reference call shape (pornhub, redtube, xtits and others):
//! ```ignore
//! let formats = expand_hls_in_place(formats, http_client).await;
//! let (formats, flags) = detect_format_sizes_lazy(formats, ctx, name).await;
//! ```
//!
//! See issue #269 for the audit and remediation history.

use std::sync::Arc;
use std::time::Duration;

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

/// Expand only the M3u8 / M3u8Native rows that do NOT yet carry `fragments`,
/// leaving every other row (already-expanded HLS, progressive, DASH) exactly
/// where it was. The orchestrator's downloader guarantee (rdlp-api's shared
/// `Orchestrator::finish_extracted_formats` boundary, called from
/// `extract_video`, `extract_lazy_formats`, and `extract_playlist`) uses this
/// so an extractor that skipped expansion — a plugin cannot return fragments
/// through the WIT `format` record — still yields downloadable HLS rows,
/// while in-tree rows (already expanded by the extractor itself) are never
/// re-fetched. Order is preserved because expansion is per-row in place.
///
/// The whole pass runs under `budget`: the rows are extractor-controlled
/// (a plugin's, in particular) and each expansion is a network round trip
/// that runs outside any per-call plugin timeout. Once the budget is spent,
/// every remaining fragments-less HLS row is dropped — a row without
/// fragments cannot be downloaded anyway — with one warning naming how many
/// were dropped; rows already expanded and rows that never needed expansion
/// are kept as they are.
pub async fn expand_missing_hls_fragments(
    formats: Vec<Format>,
    http: Arc<wreq::Client>,
    budget: Duration,
) -> Vec<Format> {
    let deadline = tokio::time::Instant::now() + budget;
    let mut out = Vec::with_capacity(formats.len());
    let mut dropped_unexpanded = 0usize;
    let mut first_dropped: Option<String> = None;
    for f in formats {
        let needs_expansion = f.fragments.is_none()
            && matches!(
                f.protocol,
                DownloadProtocol::M3u8 | DownloadProtocol::M3u8Native
            );
        if !needs_expansion {
            out.push(f);
            continue;
        }
        if dropped_unexpanded > 0 {
            // The budget is already spent; no point starting another fetch.
            dropped_unexpanded += 1;
            continue;
        }
        let url = f.url.clone();
        match tokio::time::timeout_at(deadline, expand_hls_in_place(vec![f], Arc::clone(&http)))
            .await
        {
            Ok(rows) => out.extend(rows),
            Err(_elapsed) => {
                dropped_unexpanded = 1;
                first_dropped = Some(url);
            }
        }
    }
    if let Some(url) = first_dropped {
        log::warn!(
            "HLS expansion budget of {budget:?} exhausted — dropping {dropped_unexpanded} \
             unexpanded HLS format row(s) (first: {})",
            rdlp_redact::RedactedUrl::new(&url)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ample for a loopback mockito round trip; only the budget test below
    /// deliberately runs out of time.
    const TEST_BUDGET: Duration = Duration::from_secs(30);

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

    /// Issue #660 acceptance criterion 2: one refused seed drops only its own
    /// format row — a ladder with one hostile entry must keep its good
    /// renditions and its non-HLS rows.
    ///
    /// Scope, stated so this is not mistaken for a security test: it pins the
    /// partial-drop CONTRACT, not the gate's presence. It passes with the seed
    /// gate deleted too, because the unpatched code drops the same row via a
    /// failed connect — deliberately, since the brief required reusing the
    /// existing `Err` arm rather than inventing a new disposition. The gate's
    /// provenance is what `expand::tests::seed_*_rejected` assert (on the
    /// `URI rejected:` prefix, which only `validate_resolved_url` emits); those
    /// four are the ones that go red when the gate is removed. What is new
    /// here is the surviving set: every pre-existing drop test passes a
    /// single-format `Vec`, so none of them could have caught a rejection that
    /// aborted the whole ladder.
    #[tokio::test]
    async fn rejected_seed_drops_only_its_own_format() {
        let mut server = mockito::Server::new_async().await;
        let _good = server
            .mock("GET", "/good.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXTINF:6.0,\nseg-2.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let good = Format::new(
            "hls-good",
            format!("{}/good.m3u8", server.url()),
            "m3u8",
            DownloadProtocol::M3u8,
        );
        // Link-local metadata address: refused by the seed gate, and refused
        // there rather than by a failed connect — the `expand.rs` seed tests
        // assert the `URI rejected:` provenance directly.
        let hostile = Format::new(
            "hls-hostile",
            "http://169.254.169.254/latest/meta-data/master.m3u8",
            "m3u8",
            DownloadProtocol::M3u8,
        );
        let mp4 = Format::new(
            "1080p",
            "https://h.com/x.mp4",
            "mp4",
            DownloadProtocol::Https,
        );

        let http = Arc::new(wreq::Client::new());
        let out = expand_hls_in_place(vec![good, hostile, mp4], http).await;

        let ids: Vec<&str> = out.iter().map(|f| f.format_id.as_str()).collect();
        assert!(
            !ids.iter().any(|id| id.contains("hostile")),
            "the rejected seed must be dropped; got {ids:?}"
        );
        assert_eq!(
            out.len(),
            2,
            "the good HLS rendition and the non-HLS row must survive; got {ids:?}"
        );
        assert!(
            out.iter()
                .any(|f| f.fragments.as_ref().is_some_and(|fr| fr.len() == 2)),
            "the good rendition must still be fully expanded; got {ids:?}"
        );
        assert!(
            out.iter().any(|f| f.format_id == "1080p"),
            "the non-HLS row must pass through; got {ids:?}"
        );
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

    /// The orchestrator-level guarantee (Task 7) must not re-fetch a row
    /// that already carries fragments: in-tree extractors expanded it, and a
    /// second expansion would re-fetch the playlist and could clobber
    /// per-variant labels. Only fragments-less M3u8/M3u8Native rows are sent
    /// through `expand_hls_in_place`.
    #[tokio::test]
    async fn expand_missing_skips_rows_that_already_carry_fragments() {
        let mut server = mockito::Server::new_async().await;
        let never = server
            .mock("GET", "/has.m3u8")
            .expect(0)
            .create_async()
            .await;
        let mut f = Format::new(
            "hls",
            format!("{}/has.m3u8", server.url()),
            "m3u8",
            DownloadProtocol::M3u8,
        );
        f.fragments = Some(vec![rdlp_types::Fragment {
            url: "https://cdn.example/1.ts".into(),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: Some(6.0),
            filesize: None,
        }]);
        let out =
            expand_missing_hls_fragments(vec![f], Arc::new(wreq::Client::new()), TEST_BUDGET).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].fragments.as_ref().unwrap().len(), 1);
        never.assert_async().await;
    }

    #[tokio::test]
    async fn expand_missing_expands_only_the_fragmentless_rows_and_keeps_order() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/v.m3u8")
            .with_body(crate::hls::test_support::VARIANT_MEDIA)
            .create_async()
            .await;
        let mp4 = Format::new(
            "1080p",
            "https://h.com/x.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        let hls = Format::new(
            "hls",
            format!("{}/v.m3u8", server.url()),
            "m3u8",
            DownloadProtocol::M3u8Native,
        );
        let out = expand_missing_hls_fragments(
            vec![mp4, hls],
            Arc::new(wreq::Client::new()),
            TEST_BUDGET,
        )
        .await;
        assert_eq!(
            out.iter().map(|f| f.format_id.as_str()).collect::<Vec<_>>(),
            ["1080p", "hls"]
        );
        assert!(out[0].fragments.is_none());
        assert_eq!(out[1].fragments.as_ref().unwrap().len(), 2);
    }

    /// The budget bounds the WHOLE pass, not each row: with a master that
    /// answers only after the budget is spent, the first HLS row's fetch
    /// times out, every later fragments-less HLS row is dropped without a
    /// fetch, and the rows that never needed expansion survive in order.
    /// The elapsed time pins that the pass did not wait for the slow
    /// master (or a second one) to answer.
    #[tokio::test]
    async fn budget_exhaustion_drops_the_unexpanded_hls_rows_and_keeps_the_rest() {
        const BUDGET: Duration = Duration::from_millis(200);
        const SERVER_DELAY: Duration = Duration::from_millis(1_000);

        let mut server = mockito::Server::new_async().await;
        let slow = server
            .mock("GET", "/slow.m3u8")
            .with_chunked_body(|w| {
                std::thread::sleep(SERVER_DELAY);
                w.write_all(crate::hls::test_support::VARIANT_MEDIA.as_bytes())
            })
            .expect(1)
            .create_async()
            .await;
        let never = server
            .mock("GET", "/never.m3u8")
            .with_body(crate::hls::test_support::VARIANT_MEDIA)
            .expect(0)
            .create_async()
            .await;

        let mp4 = Format::new(
            "1080p",
            "https://h.com/x.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        let slow_hls = Format::new(
            "slow",
            format!("{}/slow.m3u8", server.url()),
            "m3u8",
            DownloadProtocol::M3u8,
        );
        let later_hls = Format::new(
            "later",
            format!("{}/never.m3u8", server.url()),
            "m3u8",
            DownloadProtocol::M3u8Native,
        );
        let mut resolved = Format::new(
            "resolved",
            format!("{}/resolved.m3u8", server.url()),
            "m3u8",
            DownloadProtocol::M3u8,
        );
        resolved.fragments = Some(vec![]);

        let started = std::time::Instant::now();
        let out = expand_missing_hls_fragments(
            vec![mp4, slow_hls, later_hls, resolved],
            Arc::new(wreq::Client::new()),
            BUDGET,
        )
        .await;
        let elapsed = started.elapsed();

        assert_eq!(
            out.iter().map(|f| f.format_id.as_str()).collect::<Vec<_>>(),
            ["1080p", "resolved"],
            "both fragments-less HLS rows must be dropped; the others kept in order"
        );
        assert!(
            elapsed < SERVER_DELAY,
            "the pass must stop at the budget, not wait for the slow master: {elapsed:?}"
        );
        slow.assert_async().await;
        never.assert_async().await;
        drop(server);
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
                "extractors/pornoxo/mod.rs",
                include_str!("../extractors/pornoxo/mod.rs"),
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
