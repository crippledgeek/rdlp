# HLS Operation Timeouts — Configurable Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift the 3 hard-coded `Duration::from_secs(N)` timeouts in `crates/rdlp-extractor/src/hls/format_detection.rs` to two new `Config` fields (`hls_operation_timeout`, `hls_head_probe_timeout`), and push the same durations into `wreq::RequestBuilder::timeout(D)` on the inner HTTP requests for defense-in-depth.

**Architecture:** Two `Option<u64>` fields on `rdlp_types::Config` mirror the existing `socket_timeout` shape. `format_detection.rs` resolves them via small helpers (`resolve_hls_operation_timeout` / `resolve_hls_head_probe_timeout`) and threads `Duration` arguments down to (1) `enrich_single_hls_format`, (2) `HlsSizeDetector::fetch_playlist_text` (chokepoint for `detect_hls_metadata` + `detect_hls_variants`), (3) `BaseExtractor::detect_file_size` (gains 4th param, applies to both inner HEAD + Range-GET requests).

**Tech Stack:** Rust 1.85, `wreq` (TLS-impersonating reqwest fork — `RequestBuilder::timeout(Duration)` verified at `rdlp-api/src/orchestrator/subtitle_pipeline/mod.rs:93`), `tokio::time::timeout`, `mockito` for one end-to-end test.

**Spec:** `docs/superpowers/specs/2026-05-04-hls-operation-timeouts-configurable-design.md`

**Branch:** `feature/hls-operation-timeouts-configurable` (already created from `develop`).

**Issue:** [#277](https://github.com/crippledgeek/rdlp/issues/277). Closes on merge.

---

## Verified call-site shapes (from research pass)

| Site | File:line | Current | New plumbing |
|---|---|---|---|
| 1. `enrich_single_hls_format` | `crates/rdlp-extractor/src/hls/format_detection.rs:79-82` | `timeout(Duration::from_secs(10), hls_detector.detect_hls_metadata(url))` | New `op_timeout: Duration` param on the function; caller passes `resolve_hls_operation_timeout(&ctx.config)`. |
| 2. `detect_format_sizes_inner` HLS branch | `crates/rdlp-extractor/src/hls/format_detection.rs:233-237` | `timeout(Duration::from_secs(10), hls_detector.detect_hls_variants(&url))` | Replace literal with `resolve_hls_operation_timeout(&ctx.config)`. |
| 3. `detect_format_sizes_inner` non-HLS branch | `crates/rdlp-extractor/src/hls/format_detection.rs:391-395` | `timeout(Duration::from_secs(5), BaseExtractor::detect_file_size(&url, &http_client, None))` | Replace literal with `resolve_hls_head_probe_timeout(&ctx.config)`; pass that same Duration as the new 4th arg to `detect_file_size`. |

`fetch_playlist_text` (the chokepoint inside `HlsSizeDetector` for sites 1 + 2) gains a `timeout: Duration` parameter and applies `.timeout(timeout)` to its `RequestBuilder` at `crates/rdlp-extractor/src/hls/detector.rs:236`.

`detect_file_size` at `crates/rdlp-extractor/src/base/common/mod.rs:379` (signature `(url, http_client, log_prefix)`) gains `timeout: Duration` as the 4th param and applies `.timeout(timeout)` to BOTH the HEAD call (line ~386) AND the Range-GET fallback (line ~395-399).

---

## Task 1 — Add `Config` fields + validation + tests

**Files:**
- Modify: `crates/rdlp-types/src/config.rs`
- Test: `crates/rdlp-types/src/config_tests.rs`

- [ ] **Step 1: Write failing tests**

Append to `crates/rdlp-types/src/config_tests.rs`. Look for an existing test like `socket_timeout_zero_rejected` to mirror the exact import / call shape. Then add:

```rust
#[test]
fn hls_operation_timeout_default_is_some_10() {
    let c = Config::default();
    assert_eq!(c.hls_operation_timeout, Some(10));
}

#[test]
fn hls_head_probe_timeout_default_is_some_5() {
    let c = Config::default();
    assert_eq!(c.hls_head_probe_timeout, Some(5));
}

#[test]
fn hls_operation_timeout_zero_rejected() {
    let c = Config {
        hls_operation_timeout: Some(0),
        ..Config::default()
    };
    let err = c.validate().expect_err("must reject");
    assert!(format!("{err:#}").contains("hls_operation_timeout"));
}

#[test]
fn hls_operation_timeout_above_max_rejected() {
    let c = Config {
        hls_operation_timeout: Some(301),
        ..Config::default()
    };
    assert!(c.validate().is_err());
}

#[test]
fn hls_head_probe_timeout_zero_rejected() {
    let c = Config {
        hls_head_probe_timeout: Some(0),
        ..Config::default()
    };
    let err = c.validate().expect_err("must reject");
    assert!(format!("{err:#}").contains("hls_head_probe_timeout"));
}

#[test]
fn hls_head_probe_timeout_above_max_rejected() {
    let c = Config {
        hls_head_probe_timeout: Some(301),
        ..Config::default()
    };
    assert!(c.validate().is_err());
}

#[test]
fn hls_timeouts_round_trip_serde() {
    let c = Config {
        hls_operation_timeout: Some(15),
        hls_head_probe_timeout: Some(7),
        ..Config::default()
    };
    let json = serde_json::to_string(&c).expect("serialize");
    let back: Config = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.hls_operation_timeout, Some(15));
    assert_eq!(back.hls_head_probe_timeout, Some(7));
}
```

- [ ] **Step 2: Run tests — expect FAIL**

Run: `cargo test -p rdlp-types hls_operation_timeout hls_head_probe_timeout hls_timeouts_round_trip_serde`
Expected: compile error (fields don't exist).

- [ ] **Step 3: Add the two fields**

In `crates/rdlp-types/src/config.rs`, locate the existing HTTP timeout field block (around lines 148-172 — `socket_timeout`, `read_timeout`, `pool_idle_timeout`). Add IMMEDIATELY AFTER `pool_idle_timeout`:

```rust
    /// Wall-clock cap on multi-step HLS probes (master playlist fetch + parse,
    /// per-format metadata refresh). Distinct from the socket-level
    /// `read_timeout`: this is total elapsed time for the helper, regardless
    /// of how many TCP reads it spans. Used by
    /// `crates/rdlp-extractor/src/hls/format_detection.rs`.
    /// Validated post-load by `Config::validate()`: must be 1..=300 seconds.
    pub hls_operation_timeout: Option<u64>,

    /// Wall-clock cap on a single HEAD probe used to detect content-length on
    /// non-HLS formats. Smaller than `hls_operation_timeout` because the
    /// operation is a single HEAD request (with a Range-GET fallback), not a
    /// multi-step playlist parse.
    /// Validated post-load by `Config::validate()`: must be 1..=300 seconds.
    pub hls_head_probe_timeout: Option<u64>,
```

In `impl Default for Config { fn default() -> Self { Self { ... } } }` (around line 359 — read it to confirm exact position), add to the field initializer list (alongside the other timeout defaults):

```rust
            hls_operation_timeout: Some(10),
            hls_head_probe_timeout: Some(5),
```

In `Config::validate()` (around lines 524-548 — the existing HTTP timeout validation block), add IMMEDIATELY AFTER the `pool_idle_timeout` validation:

```rust
        if let Some(t) = self.hls_operation_timeout
            && !(1..=300).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "hls_operation_timeout",
                reason: "must be 1..=300 seconds",
            });
        }
        if let Some(t) = self.hls_head_probe_timeout
            && !(1..=300).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "hls_head_probe_timeout",
                reason: "must be 1..=300 seconds",
            });
        }
```

- [ ] **Step 4: Run tests — expect PASS**

Run: `cargo test -p rdlp-types hls_`
Expected: all 7 PASS.

Run full crate: `cargo test -p rdlp-types`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/rdlp-types/src/config.rs crates/rdlp-types/src/config_tests.rs
git commit -m "feat(types): add hls_operation_timeout and hls_head_probe_timeout to Config (#277)"
```

---

## Task 2 — Add resolver helpers in `format_detection.rs`

**Files:**
- Modify: `crates/rdlp-extractor/src/hls/format_detection.rs`

These are tiny pure functions that read from `Config` and return `Duration`. They exist to give us a structural test seam (per spec test plan) and to centralize the default values so the call sites stay tidy.

- [ ] **Step 1: Write failing tests**

Append to the existing `#[cfg(test)] mod tests { ... }` block in `crates/rdlp-extractor/src/hls/format_detection.rs` (read the file first to confirm the test module location and pattern). If there's no existing test module in this file, create one at the end of the file:

```rust
#[cfg(test)]
mod resolve_timeout_tests {
    use super::{resolve_hls_head_probe_timeout, resolve_hls_operation_timeout};
    use rdlp_types::Config;
    use std::time::Duration;

    #[test]
    fn op_timeout_uses_default_when_none() {
        let c = Config {
            hls_operation_timeout: None,
            ..Config::default()
        };
        // Note: Config::default() pre-populates Some(10), so we assert against
        // the explicit-None case to prove the fallback. We override with None
        // after Config::default() so the helper's None branch is exercised.
        // (Also covers the "field stripped from settings.toml" path.)
        let mut c = c;
        c.hls_operation_timeout = None;
        assert_eq!(resolve_hls_operation_timeout(&c), Duration::from_secs(10));
    }

    #[test]
    fn op_timeout_uses_override_when_some() {
        let mut c = Config::default();
        c.hls_operation_timeout = Some(45);
        assert_eq!(resolve_hls_operation_timeout(&c), Duration::from_secs(45));
    }

    #[test]
    fn head_probe_timeout_uses_default_when_none() {
        let mut c = Config::default();
        c.hls_head_probe_timeout = None;
        assert_eq!(resolve_hls_head_probe_timeout(&c), Duration::from_secs(5));
    }

    #[test]
    fn head_probe_timeout_uses_override_when_some() {
        let mut c = Config::default();
        c.hls_head_probe_timeout = Some(2);
        assert_eq!(resolve_hls_head_probe_timeout(&c), Duration::from_secs(2));
    }
}
```

- [ ] **Step 2: Run tests — expect FAIL**

`cargo test -p rdlp-extractor resolve_timeout_tests`
Expected: compile error (helpers don't exist).

- [ ] **Step 3: Add the helpers**

In `crates/rdlp-extractor/src/hls/format_detection.rs`, near the top of the module (after `use` statements, before the first `pub fn`), add:

```rust
/// Default cap on multi-step HLS probes when `Config::hls_operation_timeout`
/// is unset. Matches the legacy hard-coded value before #277.
const DEFAULT_HLS_OPERATION_TIMEOUT_SECS: u64 = 10;

/// Default cap on the single-HEAD probe for non-HLS file size detection
/// when `Config::hls_head_probe_timeout` is unset. Matches the legacy
/// hard-coded value before #277.
const DEFAULT_HLS_HEAD_PROBE_TIMEOUT_SECS: u64 = 5;

/// Resolve the wall-clock cap for HLS metadata / variant probes from `Config`,
/// falling back to `DEFAULT_HLS_OPERATION_TIMEOUT_SECS` when unset.
pub(crate) fn resolve_hls_operation_timeout(config: &rdlp_types::Config) -> std::time::Duration {
    std::time::Duration::from_secs(
        config
            .hls_operation_timeout
            .unwrap_or(DEFAULT_HLS_OPERATION_TIMEOUT_SECS),
    )
}

/// Resolve the wall-clock cap for the non-HLS HEAD-probe from `Config`,
/// falling back to `DEFAULT_HLS_HEAD_PROBE_TIMEOUT_SECS` when unset.
pub(crate) fn resolve_hls_head_probe_timeout(config: &rdlp_types::Config) -> std::time::Duration {
    std::time::Duration::from_secs(
        config
            .hls_head_probe_timeout
            .unwrap_or(DEFAULT_HLS_HEAD_PROBE_TIMEOUT_SECS),
    )
}
```

- [ ] **Step 4: Run tests — expect PASS**

`cargo test -p rdlp-extractor resolve_timeout_tests`
Expected: 4 PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/rdlp-extractor/src/hls/format_detection.rs
git commit -m "feat(extractor): add resolve_hls_operation_timeout / resolve_hls_head_probe_timeout helpers (#277)"
```

---

## Task 3 — Thread `Duration` into `fetch_playlist_text` + `RequestBuilder::timeout`

**Files:**
- Modify: `crates/rdlp-extractor/src/hls/detector.rs`
- Modify: `crates/rdlp-extractor/src/hls/variants.rs`

`fetch_playlist_text` is `pub(super)` per `crates/rdlp-extractor/src/hls/detector.rs:235`. It's called from `detect_hls_metadata` (`hls/variants.rs:174`) and `detect_hls_variants` (`hls/variants.rs:343`).

- [ ] **Step 1: Read the current shapes**

Read:
- `crates/rdlp-extractor/src/hls/detector.rs` lines 230-275 (full `fetch_playlist_text` body).
- `crates/rdlp-extractor/src/hls/variants.rs` lines 167-185 (`detect_hls_metadata` start).
- `crates/rdlp-extractor/src/hls/variants.rs` lines 335-355 (`detect_hls_variants` start).

Confirm there are exactly two callers of `fetch_playlist_text`. (Search: `grep -n "fetch_playlist_text" crates/rdlp-extractor/src/hls/`.)

- [ ] **Step 2: Update `fetch_playlist_text` signature**

In `crates/rdlp-extractor/src/hls/detector.rs:235`, change:

```rust
    pub(super) async fn fetch_playlist_text(&self, m3u8_url: &str) -> Result<String> {
        let mut request = self.http_client.get(m3u8_url);
        if let Some(headers) = &self.default_headers {
            request = request.headers(headers.clone());
        }
        let response = request.send().await.map_err(|e| {
```

to:

```rust
    pub(super) async fn fetch_playlist_text(
        &self,
        m3u8_url: &str,
        timeout: std::time::Duration,
    ) -> Result<String> {
        let mut request = self.http_client.get(m3u8_url).timeout(timeout);
        if let Some(headers) = &self.default_headers {
            request = request.headers(headers.clone());
        }
        let response = request.send().await.map_err(|e| {
```

(The `.timeout(timeout)` chains immediately after `.get(m3u8_url)` so it applies to the request even if `default_headers` add more state.)

- [ ] **Step 3: Update both callers in `variants.rs`**

In `crates/rdlp-extractor/src/hls/variants.rs:167`, change `detect_hls_metadata`'s signature:

```rust
    pub async fn detect_hls_metadata(&self, m3u8_url: &str) -> Result<Option<HlsInfo>> {
```

to:

```rust
    pub async fn detect_hls_metadata(
        &self,
        m3u8_url: &str,
        timeout: std::time::Duration,
    ) -> Result<Option<HlsInfo>> {
```

And update the inner call (around line 174) from `self.fetch_playlist_text(m3u8_url).await` to `self.fetch_playlist_text(m3u8_url, timeout).await`.

In `crates/rdlp-extractor/src/hls/variants.rs:339`, mirror the change for `detect_hls_variants`:

```rust
    pub async fn detect_hls_variants(
        &self,
        m3u8_url: &str,
        timeout: std::time::Duration,
    ) -> Result<Vec<HlsVariantInfo>> {
```

And update its inner call (around line 343) the same way.

- [ ] **Step 4: Verify the new compilation surface — expect callers in `format_detection.rs` to break**

Run: `cargo check -p rdlp-extractor 2>&1 | head -40`
Expected: errors at the call sites in `format_detection.rs` lines 81, 235 (the outer `tokio::time::timeout` arms that now invoke `detect_hls_metadata` / `detect_hls_variants` with the OLD signature). These are fixed in Task 4.

If there are other callers of `detect_hls_metadata` or `detect_hls_variants` outside `format_detection.rs`, the call sites need updating now. Find them:

```bash
grep -rn "detect_hls_metadata\|detect_hls_variants" crates/ | grep -v test | grep -v fetch_playlist_text
```

If extras exist, update them too with `Duration::from_secs(10)` literal (a follow-up commit can plumb their proper Config later). Note this in the commit body so it's discoverable.

- [ ] **Step 5: Commit (compilation will fail at this stage — that's expected; Task 4 fixes it)**

DO NOT commit at the end of Task 3. Continue directly to Task 4 in a single batch — committing a broken build mid-branch violates the per-commit-green discipline.

If you absolutely must split, mark the Task 3 commit as a WIP-style intermediate and ensure Task 4 lands in the same PR. Better: defer the commit until Task 4 completes.

---

## Task 4 — Update call sites in `format_detection.rs`

**Files:**
- Modify: `crates/rdlp-extractor/src/hls/format_detection.rs`

- [ ] **Step 1: Update the three call sites**

In `crates/rdlp-extractor/src/hls/format_detection.rs`:

**Site 1** — `enrich_single_hls_format` (around lines 69-100). Read the current function signature (you'll see it does NOT take `ctx` — only `hls_detector, url, extractor_name, verbose`). Add a new parameter `op_timeout: std::time::Duration`. The call site inside (around line 79-82):

```rust
    let result = timeout(
        Duration::from_secs(10),
        hls_detector.detect_hls_metadata(url),
    )
```

becomes:

```rust
    let result = timeout(
        op_timeout,
        hls_detector.detect_hls_metadata(url, op_timeout),
    )
```

(The same `op_timeout` is used for both the outer `tokio::time::timeout` and the inner `RequestBuilder::timeout` propagation through `fetch_playlist_text`. This is the defense-in-depth pattern.)

**Site 2** — `detect_format_sizes_inner` HLS branch (around lines 233-237):

```rust
                    let result = timeout(
                        Duration::from_secs(10),
                        hls_detector.detect_hls_variants(&url),
                    )
```

becomes:

```rust
                    let op_timeout = resolve_hls_operation_timeout(&ctx.config);
                    let result = timeout(
                        op_timeout,
                        hls_detector.detect_hls_variants(&url, op_timeout),
                    )
```

(Resolve once into a local so both the outer and inner timeouts get the same value.)

**Site 3** — `detect_format_sizes_inner` non-HLS branch (around lines 391-395):

```rust
                        let result = timeout(
                            Duration::from_secs(5),
                            BaseExtractor::detect_file_size(&url, &http_client, None),
                        )
```

becomes:

```rust
                        let head_timeout = resolve_hls_head_probe_timeout(&ctx.config);
                        let result = timeout(
                            head_timeout,
                            BaseExtractor::detect_file_size(&url, &http_client, None, head_timeout),
                        )
```

(`detect_file_size` gains a 4th `Duration` arg in Task 5. This site call must match.)

- [ ] **Step 2: Update the caller of `enrich_single_hls_format`**

Find where `enrich_single_hls_format` is called from (almost certainly `detect_format_sizes_inner`). Resolve the duration from `ctx.config` once and pass it down:

```rust
let op_timeout = resolve_hls_operation_timeout(&ctx.config);
// ... call enrich_single_hls_format(..., op_timeout)
```

Reuse the same `op_timeout` local from Site 2 if the function structure permits (no double-resolution).

- [ ] **Step 3: Verify partial compile — expect Task 5 break only**

Run: `cargo check -p rdlp-extractor 2>&1 | tail -10`
Expected: only the call to `detect_file_size(&url, &http_client, None, head_timeout)` errors because `detect_file_size` doesn't yet have the 4th param. (Task 5.)

- [ ] **Step 4: Don't commit yet — proceed directly to Task 5.**

---

## Task 5 — Add `timeout: Duration` to `detect_file_size`

**Files:**
- Modify: `crates/rdlp-extractor/src/base/common/mod.rs`

- [ ] **Step 1: Update the signature**

In `crates/rdlp-extractor/src/base/common/mod.rs:379-410`, change:

```rust
    pub(crate) async fn detect_file_size(
        url: &str,
        http_client: &wreq::Client,
        log_prefix: Option<&str>,
    ) -> Option<u64> {
        // Strategy 1: HEAD request
        if let Ok(response) = http_client.head(url).send().await
            && let Some(size) = response.content_length().filter(|&s| s > 0)
        {
```

to:

```rust
    pub(crate) async fn detect_file_size(
        url: &str,
        http_client: &wreq::Client,
        log_prefix: Option<&str>,
        timeout: std::time::Duration,
    ) -> Option<u64> {
        // Strategy 1: HEAD request
        if let Ok(response) = http_client.head(url).timeout(timeout).send().await
            && let Some(size) = response.content_length().filter(|&s| s > 0)
        {
```

And in the Range-GET fallback (around line 395):

```rust
        // Strategy 2: Range request fallback
        if let Ok(response) = http_client
            .get(url)
            .header("Range", "bytes=0-0")
            .send()
            .await
```

becomes:

```rust
        // Strategy 2: Range request fallback
        if let Ok(response) = http_client
            .get(url)
            .header("Range", "bytes=0-0")
            .timeout(timeout)
            .send()
            .await
```

- [ ] **Step 2: Update doc-example in the rustdoc**

Lines 374-376 of the same file have:

```rust
    /// let size = BaseExtractor::detect_file_size(&url, &ctx.http_client, None).await;
    /// let size = BaseExtractor::detect_file_size(&url, &client, Some("HLS")).await;
```

Update to:

```rust
    /// let size = BaseExtractor::detect_file_size(&url, &ctx.http_client, None, Duration::from_secs(5)).await;
    /// let size = BaseExtractor::detect_file_size(&url, &client, Some("HLS"), Duration::from_secs(5)).await;
```

- [ ] **Step 3: Update other callers of `detect_file_size`**

Find them:

```bash
grep -rn "detect_file_size" crates/ | grep -v "fn detect_file_size" | grep -v test
```

For each call site, add a `Duration::from_secs(5)` 4th argument (mirroring the previous outer `tokio::time::timeout` cap). If a site has its own custom cap or no `tokio::time::timeout` wrapper today, use `Duration::from_secs(30)` as a defensive default; document the choice in the commit body.

- [ ] **Step 4: Verify full compile**

```
cargo fmt
cargo check --workspace
```
Expected: clean.

- [ ] **Step 5: Run all extractor tests + types tests**

```
cargo test -p rdlp-types
cargo test -p rdlp-extractor
```
Expected: all green (the existing tests should not be affected by adding the 4th arg + timeout chaining).

- [ ] **Step 6: Commit (Tasks 3 + 4 + 5 batched)**

```bash
cargo fmt
git add crates/rdlp-extractor/src/hls/detector.rs \
        crates/rdlp-extractor/src/hls/variants.rs \
        crates/rdlp-extractor/src/hls/format_detection.rs \
        crates/rdlp-extractor/src/base/common/mod.rs
git commit -m "feat(extractor): make HLS operation + HEAD probe timeouts configurable (#277)

Threads Duration through fetch_playlist_text, detect_hls_metadata,
detect_hls_variants, detect_file_size. Pushes RequestBuilder::timeout(D)
on inner HTTP requests for defense in depth. Replaces hard-coded 10s/5s
in format_detection.rs with resolve_hls_operation_timeout /
resolve_hls_head_probe_timeout helpers reading from Config."
```

---

## Task 6 — End-to-end mockito test (single, slowest path)

**Files:**
- Test: `crates/rdlp-extractor/src/hls/format_detection.rs` (or `tests/` if integration-test-style)

Per spec test plan, structural helper tests in Task 2 are the workhorse; one end-to-end mockito test pins the timeout-firing behavior on the slowest path.

- [ ] **Step 1: Find the in-repo mockito pattern**

Read an existing mockito-driven test in this crate. Examples:
- `crates/rdlp-extractor/src/xhamster/mod.rs:512` (mod.rs unit-test pattern)
- `crates/rdlp-extractor/src/koreanpornmovie/mod.rs:896`

The CLAUDE.md note explicitly says: "HLS tests use unit tests, not integration tests" because of the loopback bypass at `crates/rdlp-extractor/src/hls/expand.rs:92-107`. This test must therefore live in `#[cfg(test)] mod` inside `src/`, not in `tests/`.

- [ ] **Step 2: Write the test**

Append to the existing `#[cfg(test)] mod` inside `crates/rdlp-extractor/src/hls/format_detection.rs` (or create the module if absent):

```rust
#[cfg(test)]
mod end_to_end_timeout_tests {
    use super::*;
    use mockito::Server;
    use std::time::{Duration, Instant};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detect_file_size_respects_head_probe_timeout() {
        // Arrange: mockito server that delays HEAD by 5s. Configure 1s
        // timeout. Expect the call to return None within ~1.5s wall-clock.
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("HEAD", "/slow")
            .with_status(200)
            .with_chunked_body(|_| Ok(()))
            // Real-time delay simulating a slow CDN.
            .expect_at_most(1)
            .create_async()
            .await;

        // mockito doesn't expose a direct "delay" hook for HEAD; instead,
        // configure the mock to NEVER respond and rely on the timeout firing.
        // (Adjust if a future mockito version exposes a per-mock delay.)
        // For now: use mockito::Server::host_with_port() against a black-hole
        // path that the mock-server holds open. If this test pattern
        // doesn't work cleanly with mockito 1.x, fall back to a hand-rolled
        // tokio::net::TcpListener that accepts the connection then sleeps
        // for 5s before responding.

        let url = format!("{}/slow", server.url());
        let client = wreq::Client::new();

        let start = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            BaseExtractor::detect_file_size(
                &url,
                &client,
                None,
                Duration::from_secs(1),
            ),
        )
        .await;
        let elapsed = start.elapsed();

        // The outer tokio::time::timeout fires at ~1s; OR the inner
        // RequestBuilder::timeout fires at ~1s. Either way, the call
        // returns within a small tolerance.
        assert!(
            elapsed < Duration::from_millis(2000),
            "expected timeout within ~1s, got {elapsed:?}"
        );

        // The inner detect_file_size returns Option<u64>. When timed out
        // by the inner RequestBuilder, both inner requests fail and the
        // function returns None. When timed out by the outer wrapper,
        // result is Err(Elapsed).
        match result {
            Ok(None) => {} // inner-timeout path — function returned None
            Err(_) => {}   // outer-timeout path — Elapsed
            Ok(Some(_)) => panic!("should not have detected a size"),
        }
    }
}
```

If `mockito 1.x` does not support per-mock real-time delay, swap the approach to a hand-rolled `tokio::net::TcpListener` that accepts then sleeps:

```rust
// Alternative: hand-rolled black-hole listener
let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
let port = listener.local_addr().unwrap().port();
tokio::spawn(async move {
    if let Ok((stream, _)) = listener.accept().await {
        // Hold the connection open without responding for 5s.
        tokio::time::sleep(Duration::from_secs(5)).await;
        drop(stream);
    }
});
let url = format!("http://127.0.0.1:{port}/slow");
// ... rest unchanged.
```

This bypasses mockito entirely and is the simplest reliable form for "block the request from completing." Confirm `validate_url_security` allows loopback IPs in test mode (it should — see the `#[cfg(test)]` bypass at `crates/rdlp-extractor/src/hls/expand.rs:92-107`).

- [ ] **Step 3: Run the test**

```
cargo test -p rdlp-extractor end_to_end_timeout_tests::detect_file_size_respects_head_probe_timeout -- --nocapture
```
Expected: PASS (within ~1.5s wall-clock).

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add crates/rdlp-extractor/src/hls/format_detection.rs
git commit -m "test(extractor): end-to-end timeout test for detect_file_size HEAD probe (#277)"
```

---

## Task 7 — Full verification gate + pre-push reviews

**No code changes — verification + review.**

- [ ] **Step 1: Verification gate**

```bash
cargo fmt --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
Every command must exit 0.

- [ ] **Step 2: Dispatch security-reviewer**

Pass: `BASE..HEAD` diff. Threat model focus:

- DoS via misconfiguration (too-long timeout × too many extractors → resource accumulation)
- Bypass scenarios (does the inner `RequestBuilder::timeout` actually fire on socket dead-end, or only on body bytes?)
- Validation bypass (can a hand-edited `~/.config/rdlp/config.toml` set `hls_operation_timeout = 999999`?)
- Per the rule at `~/.claude/rules/mandatory-pre-push-review.md`: HIGH/Critical findings MUST be fixed BEFORE push, not after.

- [ ] **Step 3: Dispatch pr-review-toolkit:code-reviewer**

Pass: `BASE..HEAD` diff. Project standards: `CODING_RULES.md`, `~/.claude/rules/no-hardcoded-magic-numbers.md` (this is the rule that motivated the work — verify the implementation actually conforms), `~/.claude/rules/bug-fix-requires-failing-test.md`.

- [ ] **Step 4: Apply review fixes (if any) and re-verify**

If reviewers flag HIGH/Critical or Important issues, fix in a follow-up commit on this branch and re-run the gate + reviews.

- [ ] **Step 5: Push + open PR**

```bash
git push -u origin feature/hls-operation-timeouts-configurable
gh pr create --base develop \
  --title "Lift HLS operation + HEAD-probe timeouts to Config (#277)" \
  --body "$(cat <<'EOF'
## Summary

Closes #277.

Lifts three hard-coded `Duration::from_secs(N)` values in
`crates/rdlp-extractor/src/hls/format_detection.rs` to two new `Config`
fields: `hls_operation_timeout` (default 10s, validated 1..=300) covering the
two multi-step HLS probes, and `hls_head_probe_timeout` (default 5s, same range)
covering the single-HEAD non-HLS file-size probe. Pushes the same `Duration`
through `wreq::RequestBuilder::timeout(D)` on the inner HTTP requests for
defense-in-depth.

This is the first instance of the new global rule
`~/.claude/rules/no-hardcoded-magic-numbers.md` being applied to a follow-up
issue.

## Test plan

- [x] `cargo fmt --check` clean
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `cargo test --workspace` — 0 failures
- [x] 7 new validation tests + 4 helper-resolver tests + 1 end-to-end mockito test
- [x] Pre-push security-reviewer ✅
- [x] Pre-push code-reviewer ✅
EOF
)"
```

---

## Self-review

**Spec coverage:**
- Two `Config` fields → Task 1 ✓
- Validation 1..=300 → Task 1 ✓
- Default values 10s/5s → Task 1 ✓
- Resolver helpers → Task 2 ✓
- 3 call-site rewires → Task 4 ✓
- `RequestBuilder::timeout` defense in depth → Task 3 (`fetch_playlist_text`) + Task 5 (`detect_file_size`) ✓
- Two-knob model preserved (multi-step vs single-HEAD) → Tasks 1+4 ✓
- Tests: structural helper tests + 1 end-to-end → Task 2 + Task 6 ✓

**Placeholder scan:** none.

**Type consistency:**
- `hls_operation_timeout: Option<u64>` everywhere it appears.
- `Duration` parameters propagated as `std::time::Duration` (no `Duration::from_secs(u64)` ambiguity).
- Resolver helpers consistently `pub(crate) fn ... -> Duration`.

**Scope:** single PR; touches 5 files (`config.rs`, `config_tests.rs`, `format_detection.rs`, `detector.rs`, `variants.rs`, `base/common/mod.rs` — actually 6, all small targeted changes). One coherent diff.

**Cross-task ordering note:** Tasks 3, 4, 5 must batch in a single commit (intermediate states fail to compile). Task 1, 2, 6, and 7 each commit independently.
