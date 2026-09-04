# Writing a New Extractor

This is the contributor guide for adding a new site to rdlp. It walks through the full lifecycle: probing the site, deciding what kind of extractor it is, wiring it in, and testing it. The goal is for a competent Rust developer to land their first extractor in under an hour.

> A "wiki" version of this guide may eventually live on GitHub; until then this file is the source of truth.

---

## 1. Decide whether the site needs a dedicated extractor

Before writing anything, run the URL through the **generic fallback extractor**:

```bash
cargo run -q -p rdlp-cli -- --dump-json "<url>" | jq '.extractor, (.formats | length)'
```

The generic extractor (`crates/rdlp-extractor/src/extractors/generic/`) tries 12 detection strategies — JSON-LD, OpenGraph, JW Player config, KVS flashvars, HTML5 `<video>`, etc. If it returns a usable format list and the right title, **you do not need a dedicated extractor**. Open an issue with the URL instead.

A site needs a dedicated extractor when:
- The format URL is obfuscated, signed, or fetched from a separate XHR.
- The page is rendered client-side and exposes no metadata in the initial HTML.
- The site shares infrastructure with one already supported (TNAFlix network, WGCZ network) — extend the shared base instead of forking it.

---

## 2. Probe the site with `rdlp-probe`

`rdlp-probe` is the production HTTP stack (wreq + BoringSSL with browser TLS impersonation) and the same boa JavaScript engine the live extractors use, packaged as a CLI. Use it for every research step — never `curl`, because some sites reject non-impersonated TLS fingerprints.

```bash
# Fetch a page exactly as the live extractor will see it
cargo run -q -p rdlp-probe -- fetch "<url>" > /tmp/page.html 2> /tmp/page.headers

# Switch browser profile if the default Chrome profile is blocked
cargo run -q -p rdlp-probe -- fetch --browser firefox "<url>"

# POST with a body and custom headers
cargo run -q -p rdlp-probe -- fetch -X POST -d '{"id":42}' \
    -H 'Content-Type: application/json' \
    -H 'Referer: https://site.example/' \
    "https://site.example/api/player"

# Evaluate JavaScript with the boa engine — for inline obfuscation,
# decoders, or fingerprint scripts the page injects
cargo run -q -p rdlp-probe -- eval 'JSON.stringify(["abc".split("").reverse()])'

# Apply a regex / CSS selector / JSON pointer to a captured payload
cargo run -q -p rdlp-probe -- extract --regex 'video_url["\047]?\s*:\s*["\047]([^"\047]+)' < /tmp/page.html

# Record a (request, response) pair as a JSON cassette for later replay
cargo run -q -p rdlp-probe -- record "<url>" --output cassettes/site.json
```

What to look for in the page body:

| Signal | Likely shape |
|---|---|
| Inline `var flashvars = { video_url: '…', video_alt_url: '…' }` | KVS site — see `crates/rdlp-extractor/src/base/kvs.rs` and the **XTits** extractor for the canonical adapter. |
| `<script type="application/ld+json">{ "@type": "VideoObject", … }` | JSON-LD source — use `BaseExtractor::parse_json_ld` and look at **EPorner** for the actor-extraction pattern. |
| `html5player.setVideoHLS(…)` / `html5player.setVideoUrlLow(…)` | WGCZ Holding network (XVideos, XNXX, …) — extend `WgczNetworkBase` rather than writing from scratch. |
| Empty `<title>`, no OG tags, just `window.constants = {…}` | Client-rendered SPA. Find the XHR endpoint by greping `app.js` for `/api/`. The **ABXXX** extractor is the reference for this shape. |
| Encrypted / encoded URL strings (base64 with non-ASCII bytes mixed in, or numeric keys) | Per-site decoder needed. Look at **XHamster** (boa-evaluated `window.initials`) and **ABXXX** (Cyrillic-homoglyph + comma-split base64). |

If you see neither flashvars nor a JSON-LD block, look at the JS bundle:

```bash
cargo run -q -p rdlp-probe -- fetch "https://site.example/static/js/app.js" > /tmp/app.js
grep -oE '"/(api|get|player|video)[^"]*"' /tmp/app.js | sort -u
```

The endpoint that returns the format list almost always shows up in this list.

---

## 3. Pick the right module shape

All extractors live under `crates/rdlp-extractor/src/extractors/<sitename>/`. The minimum viable layout is:

```
crates/rdlp-extractor/src/extractors/<sitename>/
├── mod.rs          # The InfoExtractor impl + glue
├── patterns.rs     # URL regex, id/slug helpers
└── decode.rs       # (optional) per-site URL/format decoder
```

Add a `search.rs` + `search_patterns.rs` only if the site exposes a public search endpoint and you want `--search-site <name>` support.

The shared building blocks you should reach for first:

- `crate::base::common::BaseExtractor::fetch_webpage` / `fetch_webpage_with_headers` — the only HTTP entry points an extractor should use. They run through the security validation gate and apply the production retry policy.
- `crate::base::common::BaseExtractor::parse_json_ld` — handles tolerant JSON-LD VideoObject parsing.
- `crate::base::kvs::*` — flashvars parser.
- `crate::base::tnaflix_network::*` and `crate::base::wgcz_network::*` — shared bases for site families.
- `crate::hls::detect_format_sizes_lazy` — concurrent file-size probing for variant lists.

---

## 4. Write the extractor

Mandatory contract (`rdlp_core::InfoExtractor`):

```rust
#[async_trait]
impl InfoExtractor for MySiteExtractor {
    fn name(&self) -> &str { "MySite" }
    fn valid_url(&self) -> &Regex { &patterns::URL_PATTERN }
    fn priority(&self) -> i32 { 50 }                // higher than 0; generic is -1000
    fn suitable(&self, url: &str) -> bool { patterns::is_suitable(url) }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // 1. Parse id/slug from the URL.
        // 2. Fetch the page (fail fast on 404, prime cookies for the API call).
        // 3. Fetch / parse the format payload.
        // 4. Build a Vec<Format> with explicit DownloadProtocol.
        // 5. Return InfoDict::new(id, title, name(), url) populated with formats + metadata.
    }
}
```

Required hygiene — these are enforced by reviewers:

- **Use typed enums everywhere.** `DownloadProtocol::Https` not `"https"`. `ContainerFormat::Mp4` not `"mp4"`. See `CODING_RULES.md`.
- **Wrap all errors with context.** Internal helpers use `anyhow::Result` with `.context("…")` chains. At the trait boundary convert to `RdlpError` with `format!("{e:#}")` so the chain survives.
- **Always populate `RdlpError::Extraction { url: Some(url.to_string()), … }`** — propagation into the API error type relies on it.
- **Set `info.age_limit`** for age-gated sites.
- **Call `info.propagate_duration()`** if you set a top-level `info.duration` so each format also carries it.
- **Validate format URLs only at the orchestrator boundary.** The orchestrator runs every URL you return through `rdlp_security::validate_url_security` — do not bypass that by handing URLs straight to a hand-rolled HTTP client inside the extractor.
- **Use `log::debug!` (not `tracing::*`) in `rdlp-extractor`.** This crate depends on `log`. Check the crate's `Cargo.toml` before adding log statements.

---

## 5. Register the extractor

Two edits, both in `crates/rdlp-extractor/src/`:

```rust
// extractors/mod.rs
pub mod mysite;
pub use mysite::MySiteExtractor;
```

```rust
// lib.rs — add to the re-export list and to ExtractorRegistry::new
pub use extractors::{… MySiteExtractor …};

registry.register(Arc::new(MySiteExtractor::new()));
// keep the GenericExtractor registration LAST — it is the lowest-priority fallback
```

---

## 6. Test it

Tier 1 — unit tests in the extractor's own `mod tests`:

- URL routing (positive + negative cases).
- Decoder logic with at least one **captured fixture** so the test does not need network access.
- Helper functions (slug humanisation, query parsing, etc.).

Tier 2 — live extraction against the real site:

```bash
cargo test -p rdlp-extractor --lib mysite          # unit tests
cargo run -q -p rdlp-cli -- --dump-json "<url>"    # end-to-end metadata
cargo run -q -p rdlp-cli -- --simulate "<url>"     # extract + select format, no download
```

Tier 3 — full pre-PR gate (`CLAUDE.md` mandates this before pushing):

```bash
cargo check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

If you used `bug-fix-requires-failing-test` patterns from a real reported issue, name your decoder fixture after the issue (`fixture_issue_207.json`) so future regressions are obvious.

---

## 7. PR checklist

- [ ] `EXTRACTORS.md` is unchanged unless you discovered a generally-useful pattern that belongs in this guide.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --workspace` passes.
- [ ] At least one fixture-backed unit test for any decoder logic.
- [ ] Live extraction screenshot or `--dump-json` snippet attached to the PR description.
- [ ] No new dependency on `axios`, Radix (`@radix-ui/*`), or anything that pulls in either transitively.
- [ ] PR title follows the conventional-commit shape (`feat(extractor): add MySite support`).
- [ ] Branch name conforms to `feature/<kebab-case>` and was branched from `develop`.

---

## Reference: existing extractors by shape

| Extractor | Shape | Read first if your site… |
|---|---|---|
| `xtits` | Inline KVS flashvars | … embeds `var flashvars = {…}` directly in HTML |
| `redtube` | API-first (Webmaster JSON) with HTML fallback | … has a public JSON API |
| `pornhub` | Inline JS player config + Webmaster search API | … has a multi-step JS player init |
| `eporner` | Authenticated XHR (`calc_hash`) + JSON-LD actors | … uses a per-page hash to authorise the format API |
| `xhamster` | boa JS evaluation of `window.initials` + per-site URL decryption | … runs an in-page decoder on encrypted format URLs |
| `xvideos` / `xnxx` | Shared `WgczNetworkBase` | … is on the WGCZ Holding network |
| `tnaflix` / `empflix` / `moviefap` | Shared `TnaFlixNetworkBase` | … is on the TNAFlix network |
| `abxxx` | Client-side SPA + obfuscated XHR (Cyrillic homoglyph + comma-split base64) | … exposes nothing in initial HTML and the format URL is encoded |
| `hqporner` | Two-layer iframe resolver (mydaddy.cc embed → direct MP4) | … proxies through a separate embed host for the real media URL |
| `nine_anime` | AJAX server-list API + Megacloud/Rapid-Cloud embed decryption | … resolves episodes through a third-party embed-decryption service |
| `koreanpornmovie` | WordPress/RetroTube `player-x.php?q=<base64>` iframe, decodes to direct MP4 or an external embed | … wraps content behind a base64-encoded plugin iframe |
| `spankbang` | Inline `stream_data = {...}` Python-dict parse, POST formats-API fallback; Cloudflare-fronted (requires TLS impersonation) | … is Cloudflare-fronted and embeds a Python-literal (not JSON) data blob |
| `pornoxo` | Signed HLS ladder in an inline `playerConfig` block, minted per page load (never cached); search is Cloudflare-gated behind a cookie-free tag-listing fallback | … signs its HLS master URL per page load, or gates search but not video pages |
| `generic` | 12-strategy fallback | … is none of the above (first try generic before writing new code) |

---

## Reference: `rdlp-probe` quick recipes

Inspect what a page actually serves with the production TLS stack:

```bash
# Status + curated headers + body
cargo run -q -p rdlp-probe -- fetch "<url>"

# All response headers
cargo run -q -p rdlp-probe -- fetch --headers "<url>"

# Pin a specific browser identifier
cargo run -q -p rdlp-probe -- fetch --browser chrome-137 "<url>"

# Pull one field from a JSON response
cargo run -q -p rdlp-probe -- fetch "https://api.example/v1/x" \
    | cargo run -q -p rdlp-probe -- extract --json-pointer /data/items/0/url
```

Always probe with `rdlp-probe` before assuming anything about a site's response shape — assumptions are the most expensive part of an extractor sprint.

---

## Writing a WASM plugin

If you don't want to add an extractor to the rdlp source tree (e.g. because you want to ship it independently, write it in a language other than Rust, or keep it private), you can build a WASM Component Model plugin instead.

Plugins implement the `extractor-plugin` WIT world declared at `crates/rdlp-plugin/wit/extractor.wit`. Any language with a working WIT bindgen toolchain can author plugins:

- **Rust** via `cargo-component`
- **Python** via `componentize-py`
- **C / C++** via `wit-bindgen c` + `wasi-sdk`
- **Go** via TinyGo with WASI Preview 2 support
- **TypeScript / JavaScript** via `jco` (ComponentizeJS)
- **Zig**, **C# / .NET**, **MoonBit** — also supported

Plugins must be signed (Sigstore keyless via GitHub Actions OIDC, or Ed25519 fallback) and dropped into the user's plugin directory (defaults to `~/.config/rdlp/plugins/<name>/`). On first run, rdlp shows the plugin's declared capabilities and asks the user to confirm trust.

Reference plugin example (in Rust + cargo-component) and full plugin author guide are tracked in [issue #213](https://github.com/crippledgeek/rdlp/issues/213) — pending Task 28. For the design rationale and security model, see `docs/superpowers/specs/2026-04-28-plugin-system-mvp-design.md` (local).

## Policy: yt-dlp-ported plugins stay byte-identical

Plugins built via `rdlp plugin build-from-ytdlp` MUST keep their `.py` source byte-identical to the upstream `yt_dlp/extractor/<name>.py` they were ported from. Local edits (regex broadening, helper substitution, behavior tweaks) are explicitly forbidden, even when the upstream source has a known defect.

Rationale:

- Drop-in compatibility is the value proposition. As soon as we modify a port we own a permanent fork of that file and lose the "paste an upstream `.py` and rebuild" property.
- yt-dlp benefits from the bug report. Upstream is the right place to fix `_VALID_URL` regex defects, missing field handling, or stale CSS selectors. Filing the issue / PR there returns the fix to ~1,800 other consumers, not just rdlp users.
- Attribution + maintenance. A locally-patched extractor becomes invisible work that we now have to keep current against upstream changes. That cost is per-extractor and compounds with every site we port.

If a ported plugin fails because of an upstream defect, the response is:

1. **Confirm the defect is upstream** — reproduce against `yt-dlp` itself if possible, or read the upstream `.py` source carefully.
2. **File the issue / PR at `yt-dlp/yt-dlp`** — link to it from the rdlp issue tracker.
3. **If the gap is on rdlp's side instead** (missing host helper, incomplete `traverse_obj` semantics, dispatcher behaviour), fix it inside rdlp — not inside the port.
4. **If a user needs the fix immediately**, they can keep a local fork of the `.py` outside the rdlp tree and run `build-from-ytdlp` on it. The plugin sandbox treats that the same as any other plugin.

The corollary: when adding a NEW in-tree (Rust-native) extractor, we are NOT prohibited from coding any site we want — that's the existing extractor authoring workflow and is fully under our control. The policy only constrains the `examples/plugins/<name>/<name>.py` files that `build-from-ytdlp` is meant to consume verbatim.
