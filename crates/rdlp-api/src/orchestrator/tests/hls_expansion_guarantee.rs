//! The downloader's HLS path refuses a `Format` without pre-resolved
//! fragments (`rdlp-downloader/src/hls/mod.rs`). In-tree extractors always
//! expand before returning; a plugin cannot, because the WIT `format` record
//! has no fragments field. `Orchestrator::extract_video`,
//! `extract_lazy_formats`, and `extract_playlist` are the three places an
//! extractor's formats can reach the downloader without another pass through
//! this boundary, so all three carry the same guarantee (see
//! `Orchestrator::expand_missing_hls_fragments` in `extraction.rs`).
//!
//! Each of the three tests below drives one of those functions through a
//! fake extractor (`test_support::FakeExtractor`) standing in for "any
//! extractor, in-tree or plugin", against four format shapes in one
//! `InfoDict`:
//!
//!   a. fragments-less `M3u8Native` at a mockito URL → expanded to fragments.
//!   b. `M3u8` that already carries fragments, at a mockito path mocked with
//!      `.expect(0)` → untouched, mock never hit (no re-fetch).
//!   c. plain `Https` progressive → untouched (not HLS at all).
//!   hostile. `M3u8` seeded at a link-local address → dropped by the SSRF
//!      gate inside `expand_missing_hls_fragments`.
//!
//! The survivors' ids and order (`a`, `b`, `c`) are asserted together, so a
//! regression that reorders or fails to drop the hostile row is caught by
//! the same assertion that catches a missed expansion.
//!
//! Every test below holds a `mockito::Server` for its full body and drops it
//! explicitly as the LAST statement, after `assert_survivors`. Same
//! convention, same reason, as `rdlp-plugin`'s
//! `host/extract_helpers/tests.rs::hls_host_imports`: `clippy::
//! significant-drop-tightening lint` would otherwise suggest dropping `server`
//! right after its last field access inside `build_fixture` (`server.mock`),
//! but `mockito::Server`'s `Drop` calls `reset()`, which clears its
//! registered mocks — doing that before the awaited `extract_*` request and
//! `assert_survivors` run would break the test, not just release a resource
//! earlier. Moving the real last use (this explicit `drop`) to the true end
//! of the function closes the gap the lint flags without changing when the
//! server actually goes away.

use crate::orchestrator::test_support::orchestrator_with_fake_extractor;
use rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA;
use rdlp_types::{DownloadProtocol, Format, Fragment, InfoDict};

/// A link-local address (RFC 3927 / the cloud-metadata range) — never
/// routable to a mockito loopback server, so a row seeded here can only
/// survive if the SSRF gate inside `expand_missing_hls_fragments` failed to
/// reject it.
const HOSTILE_HOST: &str = "169.254.169.254";

/// The `InfoDict` shared by all three tests, plus the mock handles needed to
/// keep alive until each test's own explicit `drop(server)`.
struct Fixture {
    info: InfoDict,
    already_resolved_seg_url: String,
    _fragments_less: mockito::Mock,
    _never_hit: mockito::Mock,
}

async fn build_fixture(server: &mut mockito::ServerGuard) -> Fixture {
    let base = server.url();

    // (a) fragments-less M3u8Native — must be fetched and expanded.
    let fragments_less = server
        .mock("GET", "/a.m3u8")
        .with_body(VARIANT_MEDIA)
        .create_async()
        .await;

    // (b) M3u8 already carrying fragments, at a path that must NEVER be hit.
    let never_hit = server
        .mock("GET", "/b.m3u8")
        .with_body(VARIANT_MEDIA)
        .expect(0)
        .create_async()
        .await;

    let a = Format::new(
        "a",
        format!("{base}/a.m3u8"),
        "mp4",
        DownloadProtocol::M3u8Native,
    );

    let already_resolved_seg_url = format!("{base}/already-resolved-seg.ts");
    let mut b = Format::new("b", format!("{base}/b.m3u8"), "mp4", DownloadProtocol::M3u8);
    b.fragments = Some(vec![Fragment {
        url: already_resolved_seg_url.clone(),
        byte_range: None,
        init_url: None,
        init_byte_range: None,
        duration: Some(6.0),
        filesize: None,
    }]);

    let c = Format::new(
        "c",
        "https://example.test/c.mp4",
        "mp4",
        DownloadProtocol::Https,
    );

    let hostile = Format::new(
        "hostile",
        format!("http://{HOSTILE_HOST}/x.m3u8"),
        "mp4",
        DownloadProtocol::M3u8,
    );

    let mut info = InfoDict::new("guarantee-test", "t", "test", "https://example.test/v");
    info.formats = vec![a, b, hostile, c];

    Fixture {
        info,
        already_resolved_seg_url,
        _fragments_less: fragments_less,
        _never_hit: never_hit,
    }
}

/// Assert the shared invariant against a boundary's output: the hostile row
/// is dropped, the survivors keep their order, row `a` gained fragments, row
/// `b`'s pre-existing fragment is untouched, and row `c` never gains one.
fn assert_survivors(result: &InfoDict, fixture: &Fixture) {
    let ids: Vec<&str> = result
        .formats
        .iter()
        .map(|f| f.format_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["a", "b", "c"],
        "the hostile row must be dropped and the rest kept in order"
    );

    assert_eq!(
        result.formats[0]
            .fragments
            .as_ref()
            .expect("row a must be expanded")
            .len(),
        2,
        "VARIANT_MEDIA carries two segments"
    );

    let untouched_b = &result.formats[1];
    assert_eq!(
        untouched_b
            .fragments
            .as_ref()
            .expect("row b already had fragments")
            .len(),
        1,
        "row b's pre-existing fragment must survive untouched"
    );
    assert_eq!(
        untouched_b.fragments.as_ref().unwrap()[0].url,
        fixture.already_resolved_seg_url,
        "row b must not be re-fetched or re-expanded"
    );

    assert!(
        result.formats[2].fragments.is_none(),
        "a plain progressive row is never given fragments"
    );
}

#[tokio::test]
async fn extract_video_expands_fragments_less_hls_rows_and_drops_hostile_ones() {
    let mut server = mockito::Server::new_async().await;
    let fixture = build_fixture(&mut server).await;
    let orch = orchestrator_with_fake_extractor(fixture.info.clone());

    let result = orch
        .extract_video("https://example.test/v")
        .await
        .expect("the fake extractor never errors");

    assert_survivors(&result, &fixture);
    drop(server);
}

#[tokio::test]
async fn extract_lazy_formats_expands_fragments_less_hls_rows_and_drops_hostile_ones() {
    let mut server = mockito::Server::new_async().await;
    let fixture = build_fixture(&mut server).await;
    let orch = orchestrator_with_fake_extractor(fixture.info.clone());

    let result = orch
        .extract_lazy_formats("https://example.test/v")
        .await
        .expect("the fake extractor never errors");

    assert_survivors(&result, &fixture);
    drop(server);
}

#[tokio::test]
async fn extract_playlist_expands_fragments_less_hls_rows_and_drops_hostile_ones() {
    let mut server = mockito::Server::new_async().await;
    let fixture = build_fixture(&mut server).await;
    let orch = orchestrator_with_fake_extractor(fixture.info.clone());

    let mut result = orch
        .extract_playlist("https://example.test/v")
        .await
        .expect("the fake extractor never errors");

    assert_eq!(
        result.len(),
        1,
        "the default extract_playlist wraps extract() in a Vec of one"
    );
    assert_survivors(&result.remove(0), &fixture);
    drop(server);
}
