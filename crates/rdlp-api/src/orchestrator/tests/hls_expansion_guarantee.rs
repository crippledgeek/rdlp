//! The downloader's HLS path refuses a `Format` without pre-resolved
//! fragments (`rdlp-downloader/src/hls/mod.rs` refuses a row without them).
//! In-tree extractors always expand before returning; a plugin cannot,
//! because the WIT `format` record has no fragments field. `extract_video`,
//! `extract_lazy_formats`, and `extract_playlist` are the three places an
//! extractor's formats can reach the downloader without another pass through
//! this boundary, so all three run the same
//! `Orchestrator::finish_extracted_formats` (`extraction.rs`), which stamps
//! a `Referer` from `webpage_url` first and expands HLS second — a plugin
//! cannot set its own `Referer` (no headers field in the WIT record), so a
//! Referer-gated HLS master would 403 and the row would be dropped if
//! expansion ran first.
//!
//! Each of the three tests below drives one of those functions through a
//! fake extractor (`test_support::FakeExtractor`) standing in for "any
//! extractor, in-tree or plugin", against four format shapes in one
//! `InfoDict`:
//!
//!   a. fragments-less `M3u8Native` at a mockito URL that only matches a
//!      request carrying `Referer: WEBPAGE_URL` → pins that the Referer
//!      stamp runs BEFORE expansion; with the two steps swapped, this row's
//!      master fetch does not match the mock, gets mockito's default 501,
//!      and the row is dropped instead of expanded.
//!   b. `M3u8` that already carries fragments, at a mockito path mocked with
//!      `.expect(0)` → untouched, mock never hit (no re-fetch).
//!   c. plain `Https` progressive → untouched (not HLS at all).
//!   hostile. `M3u8` seeded at an address `rdlp_extractor::hls::expand`'s own
//!      SSRF gate rejects before any fetch is attempted (see that crate's
//!      `hls::expand::tests::seed_link_local_metadata_address_rejected` for
//!      the gate's guarantee) → dropped, order preserved for the survivors.
//!      This does NOT exercise the gate itself — any `HlsExpandError` drops
//!      a row the same way — it only pins that an unresolvable HLS row is
//!      dropped and does not disturb its siblings' order.
//!
//! The survivors' ids and order (`a`, `b`, `c`) are asserted together, so a
//! regression that reorders, fails to drop the hostile row, or reverts the
//! Referer/expansion ordering is caught by the same assertion.
//!
//! Every test below holds a `mockito::ServerGuard` for its full body and
//! drops it explicitly as the LAST statement, after `assert_survivors`. Same
//! convention, same reason, as `rdlp-plugin`'s
//! `host/extract_helpers/tests.rs::hls_host_imports`: `clippy::significant_drop_tightening`
//! would otherwise suggest dropping `server` right after its last field
//! access inside `build_fixture` (`server.mock`), but dropping a
//! `ServerGuard` recycles the underlying `Server` back to mockito's server
//! pool, and that recycle step calls `reset()`, which clears every mock the
//! server had registered — doing that before the awaited `extract_*` request
//! and `assert_survivors` run would break the test, not just release a
//! resource earlier. Moving the real last use (this explicit `drop`) to the
//! true end of the function closes the gap the lint flags without changing
//! when the server actually goes away.

use crate::orchestrator::test_support::{
    fast_failing_config, orchestrator_with_fake_extractor,
    orchestrator_with_fake_extractor_and_config,
};
use rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA;
use rdlp_types::{DownloadProtocol, Format, Fragment, InfoDict};

/// The `webpage_url` every test's `InfoDict` carries, and the URL passed to
/// the boundary function under test — kept equal, and named once, because
/// the Referer stamp copies `webpage_url` verbatim and the mock in
/// `build_fixture` must match exactly what that stamp produces.
const WEBPAGE_URL: &str = "https://example.test/v";

/// An address `rdlp_extractor::hls::expand`'s own SSRF gate rejects before
/// any fetch is attempted (RFC 3927 / the cloud-metadata range) — see the
/// module doc for what seeding a row here does and does not prove.
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

    // (a) fragments-less M3u8Native. `match_header` pins the Referer-before-
    // expansion order from the module doc: a request without exactly this
    // header does not match this mock at all.
    let fragments_less = server
        .mock("GET", "/a.m3u8")
        .match_header("Referer", WEBPAGE_URL)
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

    let mut info = InfoDict::new("guarantee-test", "t", "test", WEBPAGE_URL);
    info.formats = vec![a, b, hostile, c];

    Fixture {
        info,
        already_resolved_seg_url,
        _fragments_less: fragments_less,
        _never_hit: never_hit,
    }
}

/// Assert the shared invariant against a boundary's output: the hostile row
/// is dropped, the survivors keep their order, row `a` gained fragments
/// (which also proves its Referer-matched master fetch succeeded), row `b`'s
/// pre-existing fragment is untouched, and row `c` never gains one.
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
            .expect("row a must be expanded (its master fetch must have carried the Referer)")
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
        .extract_video(WEBPAGE_URL)
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
        .extract_lazy_formats(WEBPAGE_URL)
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
        .extract_playlist(WEBPAGE_URL)
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

/// `Config::hls_expansion_timeout` is read at the call site, not merely
/// stored: with a 1 s budget and a master that answers only after 3 s, the
/// fragments-less HLS row is dropped and `extract_video` returns before the
/// master would have answered. Under the 60 s default the same fixture
/// waits for the slow master and keeps the row.
#[tokio::test]
async fn hls_expansion_timeout_override_is_read_at_the_boundary() {
    use std::time::{Duration, Instant};

    const SERVER_DELAY: Duration = Duration::from_secs(3);

    let mut server = mockito::Server::new_async().await;
    let _slow = server
        .mock("GET", "/slow.m3u8")
        .with_chunked_body(|w| {
            std::thread::sleep(SERVER_DELAY);
            w.write_all(VARIANT_MEDIA.as_bytes())
        })
        .create_async()
        .await;

    let slow = Format::new(
        "slow",
        format!("{}/slow.m3u8", server.url()),
        "mp4",
        DownloadProtocol::M3u8Native,
    );
    let plain = Format::new(
        "c",
        "https://example.test/c.mp4",
        "mp4",
        DownloadProtocol::Https,
    );
    let mut info = InfoDict::new("budget-test", "t", "test", WEBPAGE_URL);
    info.formats = vec![slow, plain];

    // `read_timeout` is raised above `SERVER_DELAY` so the only thing that
    // can cut the slow fetch short is the expansion budget — with the
    // fast-failing 2 s read timeout, the client's own timeout would drop the
    // row on its own and this test would pass with the budget ignored.
    let config = rdlp_types::Config {
        hls_expansion_timeout: Some(1),
        read_timeout: Some(10),
        ..fast_failing_config()
    };
    let orch = orchestrator_with_fake_extractor_and_config(info, config);

    let started = Instant::now();
    let result = orch
        .extract_video(WEBPAGE_URL)
        .await
        .expect("the fake extractor never errors");
    let elapsed = started.elapsed();

    let ids: Vec<&str> = result
        .formats
        .iter()
        .map(|f| f.format_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["c"],
        "the unexpanded HLS row must be dropped when the configured budget runs out"
    );
    assert!(
        elapsed < SERVER_DELAY,
        "the configured 1 s budget must cut the pass short; took {elapsed:?}"
    );
    drop(server);
}
