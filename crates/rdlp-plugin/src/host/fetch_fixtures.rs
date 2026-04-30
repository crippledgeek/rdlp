//! Test-only fixture-replay harness for the `host:fetch` capability.
//!
//! Plugin golden tests need deterministic responses for URLs the plugin
//! would otherwise hit live (e.g. SVT's geo-restricted Sweden API). The
//! fixture map intercepts an exact URL → canned response BEFORE any real
//! HTTP call is made; misses fall through to the wreq client as normal.
//!
//! Threading model: fixtures are wrapped in `Arc` so a single shared
//! map can be cloned per plugin invocation cheaply. Lookup is O(1)
//! HashMap by exact URL — glob support deferred until the first real
//! plugin needs it (SVT has at most ~5 fixture URLs, all exact).
//!
//! Production note: `HostResources::fetch_fixtures` is `None` by default
//! in shipped binaries. Setting it requires explicit code that no
//! production code path takes — the field exists purely for the
//! per-plugin golden test harness.

use std::collections::HashMap;
use std::sync::Arc;

/// A canned HTTP response served when an inbound URL matches a fixture.
#[derive(Clone, Debug)]
pub struct FixtureResponse {
    /// HTTP status code returned to the plugin.
    pub status: u16,
    /// Response headers — same `(name, value)` tuple list shape that the
    /// real `host:fetch` impl produces.
    pub headers: Vec<(String, String)>,
    /// Response body bytes. For text fixtures, the test typically does
    /// `body: include_bytes!("fixtures/page.html").to_vec()`.
    pub body: Vec<u8>,
}

impl FixtureResponse {
    /// Build a 200 OK fixture from a body. Most golden-test cases want
    /// this with no extra headers.
    #[must_use]
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// Build a fixture with an explicit status code (e.g. 403 for
    /// geo-restriction simulation).
    #[must_use]
    pub fn with_status(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }
}

/// URL → canned-response map. Lookup is exact-string match.
#[derive(Clone, Default, Debug)]
pub struct FetchFixtures {
    map: HashMap<String, FixtureResponse>,
}

impl FetchFixtures {
    /// Empty fixture set. The `host:fetch` impl treats this the same as
    /// `None` — every fetch falls through to the real wreq client.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fixture for `url`. Replaces any prior fixture at the
    /// same URL.
    pub fn insert(&mut self, url: impl Into<String>, response: FixtureResponse) {
        self.map.insert(url.into(), response);
    }

    /// Builder: insert and return self. Convenient for one-line fixture
    /// chains in tests:
    /// ```ignore
    /// let fx = FetchFixtures::new()
    ///     .with("https://api.example/v/abc", FixtureResponse::ok(b"{}"))
    ///     .with("https://api.example/v/def", FixtureResponse::ok(b"{}"));
    /// ```
    #[must_use]
    pub fn with(mut self, url: impl Into<String>, response: FixtureResponse) -> Self {
        self.insert(url, response);
        self
    }

    /// Look up a canned response by URL. Returns `None` for misses; the
    /// caller falls through to the live wreq client in that case.
    #[must_use]
    pub fn get(&self, url: &str) -> Option<&FixtureResponse> {
        self.map.get(url)
    }

    /// Number of fixtures registered. Tests that want to confirm "every
    /// fixture was hit" can compare this against an external counter.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when no fixtures are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Newtype around `Option<Arc<FetchFixtures>>` for plumbing through
/// `HostResources` and `FetchCtx`. Cheap to clone (Arc ref-count).
pub type SharedFixtures = Option<Arc<FetchFixtures>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_fixtures_returns_none_for_any_url() {
        let fx = FetchFixtures::new();
        assert!(fx.is_empty());
        assert!(fx.get("https://anything").is_none());
    }

    #[test]
    fn exact_url_match_returns_fixture() {
        let mut fx = FetchFixtures::new();
        fx.insert(
            "https://api.example/v/abc",
            FixtureResponse::ok(b"hello".to_vec()),
        );
        let got = fx.get("https://api.example/v/abc").unwrap();
        assert_eq!(got.status, 200);
        assert_eq!(got.body, b"hello");
    }

    #[test]
    fn url_miss_returns_none() {
        let fx = FetchFixtures::new().with(
            "https://api.example/v/abc",
            FixtureResponse::ok(b""),
        );
        assert!(fx.get("https://api.example/v/different").is_none());
    }

    #[test]
    fn builder_chains_fixtures() {
        let fx = FetchFixtures::new()
            .with("https://a", FixtureResponse::ok(b"A".to_vec()))
            .with("https://b", FixtureResponse::ok(b"B".to_vec()));
        assert_eq!(fx.len(), 2);
        assert_eq!(fx.get("https://a").unwrap().body, b"A");
        assert_eq!(fx.get("https://b").unwrap().body, b"B");
    }

    #[test]
    fn duplicate_url_replaces() {
        let mut fx = FetchFixtures::new();
        fx.insert("https://x", FixtureResponse::ok(b"first".to_vec()));
        fx.insert("https://x", FixtureResponse::ok(b"second".to_vec()));
        assert_eq!(fx.len(), 1);
        assert_eq!(fx.get("https://x").unwrap().body, b"second");
    }

    #[test]
    fn with_status_explicit_code() {
        let resp = FixtureResponse::with_status(403, b"forbidden".to_vec());
        assert_eq!(resp.status, 403);
        assert_eq!(resp.body, b"forbidden");
    }
}
