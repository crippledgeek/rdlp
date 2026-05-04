# HLS Operation Timeouts — Configurable Design

**Issue:** [#277](https://github.com/crippledgeek/rdlp/issues/277)
**Date:** 2026-05-04
**Status:** Draft (pending review)

## Summary

`crates/rdlp-extractor/src/hls/format_detection.rs` has three `tokio::time::timeout` call sites with hard-coded `Duration::from_secs(N)` values: two at 10s wrapping HLS metadata / variant probes, and one at 5s wrapping a HEAD-request probe for non-HLS file size. These are wall-clock operation deadlines — semantically distinct from the socket-level `socket_timeout` / `read_timeout` / `pool_idle_timeout` axes already exposed.

Per `~/.claude/rules/no-hardcoded-magic-numbers.md`, every numeric literal that affects runtime behavior must live in `Config` (preferred) or a documented module-level `const`. This spec lifts the three values into two new `Config` fields and adds defense-in-depth at the HTTP layer via `RequestBuilder::timeout`.

## Non-goals

- No new CLI flag or Settings UI. These are low-frequency operator knobs; TOML-only is the right surface for now.
- No removal of the existing `tokio::time::timeout` wrappers — they remain as the hard cancellation wall.
- No change to the underlying `HlsSizeDetector` / `BaseExtractor::detect_file_size` APIs beyond passing a `Duration` parameter.

## Background

### Current state — three magic numbers

`crates/rdlp-extractor/src/hls/format_detection.rs`:

| Line | Site | Current cap | Wraps |
|------|------|-------------|-------|
| 79   | `enrich_single_hls_format` | `Duration::from_secs(10)` | `hls_detector.detect_hls_metadata(url)` (per-format metadata refresh) |
| 233  | `detect_format_sizes_inner` (HLS branch) | `Duration::from_secs(10)` | `hls_detector.detect_hls_variants(&url)` (master playlist fetch + parse) |
| 391  | `detect_format_sizes_inner` (non-HLS branch) | `Duration::from_secs(5)` | `BaseExtractor::detect_file_size(&url, &http_client, None)` (single HEAD request) |

### Researcher findings (cited in PR brainstorm)

- `tokio::time::timeout(D, future)` is the idiomatic Rust pattern for operation-level wall-clock caps. Cancellation drops the wrapped future at the next `.await` yield — for network I/O this is reliable.
- `reqwest::RequestBuilder::timeout(D)` is the direct analog of OkHttp `callTimeout` and covers connect-through-body. `wreq` inherits this API.
- Reusing `read_timeout` (per-read idle) as an operation cap is semantically wrong — a CDN that drips bytes within the read budget can keep a request alive for `unbounded_read_count × read_timeout`.
- No widely-distributed Rust HLS/DASH extractor crate exposes a configurable operation-level timeout with its own named field — this project is setting precedent.
- Two-knob design (multi-step vs single-HEAD) preserves the SLA distinction the original constants encoded. One-knob design loses it.

## Design

### New `Config` fields

`crates/rdlp-types/src/config.rs`:

```rust
/// Wall-clock cap on multi-step HLS probes (master playlist fetch + parse,
/// per-format metadata refresh). Distinct from the socket-level
/// `read_timeout`: this is total elapsed time for the helper, regardless of
/// how many TCP reads it spans.
/// Validated post-load by `Config::validate()`: must be 1..=300 seconds.
pub hls_operation_timeout_secs: Option<u64>,

/// Wall-clock cap on a single HEAD probe used to detect content-length on
/// non-HLS formats. Smaller than `hls_operation_timeout_secs` because the
/// operation is a single HEAD request, not a multi-step playlist parse.
/// Validated post-load by `Config::validate()`: must be 1..=300 seconds.
pub hls_head_probe_timeout_secs: Option<u64>,
```

Defaults match current constants:

```rust
hls_operation_timeout_secs: Some(10),
hls_head_probe_timeout_secs: Some(5),
```

### Validation (extend `Config::validate()`)

Mirror the existing 3-axis HTTP timeout pattern at `crates/rdlp-types/src/config.rs:524-548`:

```rust
if let Some(t) = self.hls_operation_timeout_secs
    && !(1..=300).contains(&t)
{
    return Err(ConfigValidationError::OutOfRange {
        field: "hls_operation_timeout_secs",
        reason: "must be 1..=300 seconds",
    });
}
if let Some(t) = self.hls_head_probe_timeout_secs
    && !(1..=300).contains(&t)
{
    return Err(ConfigValidationError::OutOfRange {
        field: "hls_head_probe_timeout_secs",
        reason: "must be 1..=300 seconds",
    });
}
```

### Call-site rewiring

`crates/rdlp-extractor/src/hls/format_detection.rs` reads the durations from `ctx.config` (or whatever the extractor context exposes — to be confirmed during implementation). Each call site replaces its hard-coded `Duration::from_secs(N)` with the configured value.

**Defense in depth:** in addition to the outer `tokio::time::timeout` wrapper, the inner `wreq::RequestBuilder` for each HTTP request inside `HlsSizeDetector::detect_hls_metadata` / `detect_hls_variants` and inside `BaseExtractor::detect_file_size` MUST receive `.timeout(<same duration>)`. The outer `tokio::time::timeout` remains the hard cancellation wall; the inner `RequestBuilder::timeout` triggers `wreq`'s typed timeout error (better diagnostics, cleaner socket teardown).

If pushing the duration through to `detect_file_size` requires extending its signature (currently `(&url, &http_client, None)`), do so — the third argument already exists for "an optional something"; if it is currently a header-map, add a fourth `Duration` parameter or wrap both into a struct.

### Naming rationale

- `hls_operation_timeout_secs` — the *operation* qualifier names the SLA boundary explicitly (vs `read_timeout` which is per-byte idle).
- `hls_head_probe_timeout_secs` — names both the protocol verb and the purpose, making the smaller default value self-explaining.
- `_secs` suffix matches existing `Config` convention (`socket_timeout`, `read_timeout`, `pool_idle_timeout` use the bare name; the `rdlp-http` `HttpClientConfig` uses `_secs`. The naming inconsistency in the existing code is out of scope to fix here — match whichever the surrounding `Config` block uses for the timeout family. Verify during implementation).

**Decision (locked):** match the bare-name convention used by the existing timeout fields in `rdlp-types::Config`. Field names will be `hls_operation_timeout` and `hls_head_probe_timeout` (without `_secs` suffix). Documentation cites the unit explicitly.

## Failure modes & error mapping

When a `tokio::time::timeout` fires:

- Currently: returns `Err(_)` from the wrapped future, which is mapped by the call site into a generic extraction error.
- After: same behavior. The new `RequestBuilder::timeout` triggers `wreq::Error::is_timeout() == true` BEFORE the outer wrapper fires, surfacing a typed timeout. Either path leaves the format with `filesize: None` (existing behavior); the surrounding code is already lazy-tolerant.

No changes to the public error types.

## Testing

Per `~/.claude/rules/bug-fix-requires-failing-test.md`:

### Unit tests in `format_detection.rs`

Use `tokio::time::pause` + a fixture `HlsSizeDetector` that sleeps longer than the configured timeout:

1. **Positive — `detect_format_sizes_inner` HLS branch respects override.** Configure `hls_operation_timeout = Some(2)`, fixture sleeps 5s, assert the call returns within ~2s.
2. **Positive — `enrich_single_hls_format` respects override.** Same shape, different call site.
3. **Positive — `detect_file_size` HEAD probe respects override.** Configure `hls_head_probe_timeout = Some(1)`, fixture sleeps 3s, assert the call returns within ~1s.
4. **Default still 10s/5s when unset.** Configure both as `None`, assert the duration passed to `tokio::time::timeout` matches the documented defaults (this is a structural test — may require exposing a helper that returns the resolved duration, or a logging assertion).

### Validation tests in `config_tests.rs`

5. `hls_operation_timeout = Some(0)` → `Config::validate()` returns `OutOfRange { field: "hls_operation_timeout", .. }`.
6. `hls_operation_timeout = Some(301)` → same.
7. `hls_head_probe_timeout = Some(301)` → `OutOfRange { field: "hls_head_probe_timeout", .. }`.
8. Both fields default to `Some(10)` / `Some(5)` after `Config::default()`.
9. Both fields round-trip through serde.
10. Legacy TOML without the keys deserializes cleanly with the documented defaults (forward-compat).

### Integration / regression

11. `cargo test -p rdlp-extractor` — full suite green, no regressions.
12. `cargo test -p rdlp-types` — full suite green.

## Files touched

| File | Reason |
|------|--------|
| `crates/rdlp-types/src/config.rs` | Add 2 fields, defaults, validation. |
| `crates/rdlp-types/src/config_tests.rs` | 4-6 new tests. |
| `crates/rdlp-extractor/src/hls/format_detection.rs` | Read from config, plumb to 3 call sites. Pass duration to `RequestBuilder::timeout` on inner requests. |
| `crates/rdlp-extractor/src/base/common/file_size.rs` (or wherever `detect_file_size` lives) | Extend signature to accept duration; pass to `RequestBuilder::timeout`. |
| `crates/rdlp-extractor/src/hls/detector.rs` (or wherever `HlsSizeDetector` lives) | Pass duration to `RequestBuilder::timeout` on its inner HTTP calls. |

## Open questions (resolved)

| Question | Resolution |
|---|---|
| One knob vs two? | Two — semantically distinct operations. |
| `_secs` suffix? | No — match existing `Config` bare-name convention. |
| CLI flag / Settings UI? | Out of scope — TOML-only. |
| Push duration to `RequestBuilder::timeout`? | Yes — defense in depth. |
| Extend `detect_file_size` signature? | Yes if needed. |

## References

- Issue [#277](https://github.com/crippledgeek/rdlp/issues/277)
- PR #282 (closed #278; established the timeout-field naming and validation pattern this spec mirrors)
- [tokio::time::timeout docs](https://docs.rs/tokio/latest/tokio/time/fn.timeout.html)
- [reqwest RequestBuilder::timeout docs](https://docs.rs/reqwest/latest/reqwest/struct.RequestBuilder.html)
- [Oxide RFD 400 — Cancel Safety in Async Rust](https://rfd.shared.oxide.computer/rfd/0400)
- `~/.claude/rules/no-hardcoded-magic-numbers.md` — governance rule that motivated this work
