//! ABI compat suite for the committed 0.5.2 example component
//! (`tests/fixtures/example-extractor-0.5.2`, built from
//! `examples/plugins/example-extractor` — see its README).
//!
//! Every test here goes through the PRODUCTION loader
//! (`test_support::discover_signed_after` → `Loader::discover`), never the
//! unit-test `fixture_extractor_from` shortcut, so signature, WIT-version
//! and trust-store checks are all on the path. The positive half proves the
//! 0.5.2 exports are reached by name on the frozen host world and their
//! payloads land in `InfoDict`; the refusal half proves the same artefact
//! is refused the moment its signature, bytes, or declared contract
//! version stop being what the loader accepts — run against BOTH committed
//! fixtures (0.5.0 and 0.5.2) through one helper, so neither has a private
//! copy of the refusal mechanism.
//!
//! Playlist-side behaviour of the fixture by URL (fallback, strike,
//! propagate, real listing) is pinned in the crate's unit tests
//! (`src/playlist_adapter/tests.rs`), which load the same bytes.

// Integration tests under `tests/*.rs` are not seen as `#[cfg(test)]` by
// clippy's `allow-unwrap-in-tests` (rust-clippy#13981); this is the one
// file-scope allow that policy permits.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use rdlp_core::{ExtractionContext, InfoExtractor};
use rdlp_plugin::PluginError;
use rdlp_plugin::test_support::{
    SignedPluginSpec, discover_signed_after, extraction_ctx, load_signed_adapter,
};
use tempfile::TempDir;

/// The example URL whose `extract-with-metadata` answers a full 0.5.2
/// payload (see the example's `lib.rs`).
const EXAMPLE_VIDEO_URL: &str = "https://example.com/video/1";

// ── positive: the 0.5.2 exports are reached and their payloads lift ──────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fixture_0_5_2_loads_and_extract_with_metadata_lifts() {
    let td = TempDir::new().unwrap();
    let adapter = load_signed_adapter(td.path(), &SignedPluginSpec::example_0_5_2());

    let info = adapter
        .extract(EXAMPLE_VIDEO_URL, &extraction_ctx())
        .await
        .expect("the 0.5.2 fixture extracts on the current host");

    // The frozen core still arrives (`extract` = `extract-with-metadata(url).core`).
    assert_eq!(info.id, "1");
    assert_eq!(info.title, "Example Video 1");
    // The 0.5.2 extra: typed fields and BOTH extras (the cap test below
    // relies on there being two under the default caps).
    assert_eq!(info.actors, ["Example Actor"]);
    assert_eq!(info.age_limit, Some(18));
    assert_eq!(info.extra["studio"], "Example Studio");
    assert_eq!(info.extra["series"], "Example Series");
    assert_eq!(info.extra.len(), 2, "{:?}", info.extra);
    assert_eq!(adapter.test_trap_count(), 0);
}

/// `extract-with-metadata` answering `err(unsupported-url)` maps to the
/// same `PluginError::UnsupportedUrl` the frozen `extract` path produces —
/// an `RdlpError::Extraction` whose message names it — and is a domain
/// outcome, so it is NOT a strike (Task 6 review obligation).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extract_with_metadata_domain_error_maps_without_a_strike() {
    let td = TempDir::new().unwrap();
    let adapter = load_signed_adapter(td.path(), &SignedPluginSpec::example_0_5_2());

    let err = adapter
        .extract("https://example.com/video/unsupported", &extraction_ctx())
        .await
        .expect_err("the fixture declines this URL from extract-with-metadata");

    let expected = PluginError::UnsupportedUrl {
        plugin: "example".into(),
        detail: "https://example.com/video/unsupported".into(),
    };
    assert!(
        err.to_string().contains(&expected.to_string()),
        "the extraction error must carry the UnsupportedUrl message: {err}"
    );
    assert_eq!(adapter.test_trap_count(), 0, "a domain error never strikes");
}

/// A `Config` cap reaches the extras validator end-to-end through the
/// `store.data_mut().metadata_caps` seam (Task 7 review obligation): with
/// `max_metadata_extras = Some(1)` exactly one of the fixture's two extras
/// survives — the first one listed — where the default-cap test above
/// keeps both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_extras_cap_reaches_the_validator_end_to_end() {
    let td = TempDir::new().unwrap();
    let adapter = load_signed_adapter(td.path(), &SignedPluginSpec::example_0_5_2());
    let ctx = ExtractionContext {
        config: Arc::new(rdlp_types::Config {
            max_metadata_extras: Some(1),
            ..Default::default()
        }),
        ..extraction_ctx()
    };

    let info = adapter
        .extract(EXAMPLE_VIDEO_URL, &ctx)
        .await
        .expect("a refused extra never fails the extraction");

    assert_eq!(info.extra["studio"], "Example Studio");
    assert!(
        !info.extra.contains_key("series"),
        "the second extra must be dropped by the cap: {:?}",
        info.extra
    );
    assert_eq!(info.extra.len(), 1);
    assert_eq!(adapter.test_trap_count(), 0);
}

/// The real-listing URL drives the whole `PagedPlaylist` scaffold through
/// the production loader: two pages (page 1 `has-more = true`, page 2
/// `has-more = false`) whose entries the fixture's own `extract` resolves,
/// positions stamped 1-indexed across the pages.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_playlist_resolves_every_entry_across_both_pages() {
    let td = TempDir::new().unwrap();
    let adapter = load_signed_adapter(td.path(), &SignedPluginSpec::example_0_5_2());

    let out = adapter
        .extract_playlist("https://example.com/a-real-playlist", &extraction_ctx())
        .await
        .expect("a real playlist resolves");

    let ids: Vec<&str> = out.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["1", "2", "3"], "page 1 lists 1 and 2, page 2 lists 3");
    let positions: Vec<Option<usize>> = out.iter().map(|i| i.playlist_index).collect();
    assert_eq!(positions, [Some(1), Some(2), Some(3)]);
    assert_eq!(adapter.test_trap_count(), 0);
}

// ── refusals: both committed fixtures through one helper ─────────────────

/// Load every committed fixture after `tamper` has edited its signed
/// directory, and return each fixture's label with the error the loader
/// refused it with. A fixture the loader ACCEPTS after tampering fails
/// here — refusal is the whole point.
fn refusals_after(tamper: impl Fn(&Path) + Copy) -> Vec<(&'static str, PluginError)> {
    let fixtures = [
        ("0.5.0", SignedPluginSpec::example()),
        ("0.5.2", SignedPluginSpec::example_0_5_2()),
    ];
    fixtures
        .into_iter()
        .map(|(label, spec)| {
            let td = TempDir::new().unwrap();
            let (_engine, outcome) = discover_signed_after(td.path(), &spec, tamper);
            let err = match outcome {
                Ok(_) => panic!("the tampered {label} fixture must be refused"),
                Err((_, err)) => err,
            };
            (label, err)
        })
        .collect()
}

/// Rewrite one line of the signed `plugin.toml`: the line starting with
/// `prefix` becomes `replacement`. Panics if no line matches — a tamper
/// that changed nothing would make a refusal test vacuous.
///
/// Sync `std::fs` is the test-fixture carve-out (c) in `clippy.toml`: this
/// runs in a plain closure between write and discover, never on an async
/// path.
#[allow(clippy::disallowed_methods)]
fn rewrite_manifest_line(dir: &Path, prefix: &str, replacement: &str) {
    let path = dir.join("plugin.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut hit = false;
    let rewritten: Vec<&str> = text
        .lines()
        .map(|line| {
            if line.starts_with(prefix) {
                hit = true;
                replacement
            } else {
                line
            }
        })
        .collect();
    assert!(hit, "no `{prefix}` line in the written manifest:\n{text}");
    std::fs::write(&path, rewritten.join("\n")).unwrap();
}

/// Flip the LAST byte of the signed `plugin.wasm` — past the wasm header,
/// so the file still parses as a component and the refusal is the
/// signature's doing rather than the parser's. Same `clippy.toml`
/// carve-out (c) as [`rewrite_manifest_line`].
#[allow(clippy::disallowed_methods)]
fn flip_last_wasm_byte(dir: &Path) {
    let path = dir.join("plugin.wasm");
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&path, bytes).unwrap();
}

/// A signature no key ever produced: 64 zero bytes, base64. What a plugin
/// directory looks like when `rdlp plugin sign` was never run over a
/// placeholder-free manifest.
const NEVER_SIGNED: &str =
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsigned_0_5_2_component_is_refused() {
    for (label, err) in refusals_after(|dir| {
        rewrite_manifest_line(
            dir,
            "signature = ",
            &format!("signature = \"{NEVER_SIGNED}\""),
        );
    }) {
        assert!(
            matches!(err, PluginError::SignatureInvalid { .. }),
            "{label}: expected SignatureInvalid, got {err:?}"
        );
    }
}

/// Flip one byte of `plugin.wasm` after signing: the ed25519 signature
/// covers `canonical_bytes(manifest) || wasm`, so the component bytes are
/// bound to the manifest's signature, not just the manifest text.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tampered_wasm_is_refused() {
    for (label, err) in refusals_after(flip_last_wasm_byte) {
        assert!(
            matches!(err, PluginError::SignatureInvalid { .. }),
            "{label}: expected SignatureInvalid, got {err:?}"
        );
    }
}

/// Edit a signed manifest field (`priority`) after signing: the canonical
/// bytes the signature covers change, so the unchanged signature no longer
/// verifies.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tampered_manifest_is_refused() {
    for (label, err) in refusals_after(|dir| {
        rewrite_manifest_line(dir, "priority = ", "priority = 151");
    }) {
        assert!(
            matches!(err, PluginError::SignatureInvalid { .. }),
            "{label}: expected SignatureInvalid, got {err:?}"
        );
    }
}

/// A manifest claiming a NEWER patch (`0.5.3`) than this host's contract
/// is refused at `discover`, before any crypto or compilation — for both
/// the 0.5.0 and the 0.5.2 component alike (the check reads the manifest,
/// not the component).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wit_version_0_5_3_manifest_is_refused() {
    for (label, err) in refusals_after(|dir| {
        rewrite_manifest_line(dir, "wit_version = ", "wit_version = \"0.5.3\"");
    }) {
        assert!(
            matches!(err, PluginError::WitVersionMismatch { .. }),
            "{label}: expected WitVersionMismatch, got {err:?}"
        );
    }
}

/// The 0.5.0 fixture is still the positive control for the refusal
/// helper: untampered, it loads — so a refusal above is the tamper's
/// doing, not the helper's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn untampered_0_5_0_fixture_still_loads_through_the_same_helper() {
    let td = TempDir::new().unwrap();
    let adapter = load_signed_adapter(td.path(), &SignedPluginSpec::example());
    let info = adapter
        .extract(EXAMPLE_VIDEO_URL, &extraction_ctx())
        .await
        .expect("the untampered 0.5.0 fixture loads and extracts");
    assert_eq!(info.id, "1");
    assert_eq!(info.extra.len(), 0, "a 0.5.0 component carries no extras");
}
