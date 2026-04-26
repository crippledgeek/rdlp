# rdlp-probe

Authoring toolkit for rdlp extractors.

`rdlp-probe` is a tiny CLI that wraps the same code paths the production extractors use:

- **HTTP**: `rdlp-http::HttpClientFactory` — wreq + statically-linked BoringSSL with browser TLS impersonation. Every request carries a Chrome / Firefox / Safari JA4 fingerprint by default.
- **JavaScript**: `rdlp-jsinterp::BoaJsEngine` — the in-tree boa engine the live extractors call to evaluate obfuscated player JS.
- **Cookies**: `wreq::cookie::Jar` — same jar shape `rdlp-cookies` builds for the orchestrator.

If `rdlp-probe` can pull something off a site, the live extractor will too. If `rdlp-probe` is blocked, the live extractor will be blocked the same way.

## Build & run

`rdlp-probe` is **not** in the workspace `default-members`, so a normal `cargo build` skips it. Build it explicitly:

```bash
cargo build --release -p rdlp-probe
./target/release/rdlp-probe --help
```

Or `cargo run --release -p rdlp-probe -- <subcommand>`.

## Subcommands

### `fetch` — request a URL

```bash
rdlp-probe fetch <url> [-X METHOD] [-H 'Name: value'] [-d BODY] [--browser chrome|firefox|safari|chrome-137] [--headers]
```

- stdout: response body
- stderr: status line + curated response headers (`content-type`, `cf-ray`, `server`, `cf-cache-status`, `cf-mitigated`, `set-cookie`, `location`); `--headers` prints all of them.

```bash
# Dump a video page
rdlp-probe fetch 'https://example.com/video/123' -H 'Cookie: country=US' > page.html

# POST to a JSON API
rdlp-probe fetch -X POST \
  -H 'X-Requested-With: XMLHttpRequest' \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  -d 'id=abc&data=0' \
  'https://example.com/api/videos/stream'

# Try a different browser fingerprint
rdlp-probe fetch <url> --browser firefox
```

### `eval` — run JavaScript through boa

```bash
rdlp-probe eval <script.js>                   # file
rdlp-probe eval --inline 'JSON.parse(s).id'   # inline expression
rdlp-probe eval --stdin < snippet.js          # pipe
rdlp-probe eval <script.js> --context state.json   # globals.context = <state>
```

Result is printed as pretty JSON. Use this to validate a deobfuscation routine before wiring it into an extractor.

### `extract` — apply a regex / CSS / JSON pattern to stdin or a file

```bash
rdlp-probe extract --mode regex 'data-streamkey="([^"]+)"' < page.html
rdlp-probe extract --mode css   'h1.title' < page.html
rdlp-probe extract --mode json     '/response_body' --file cassette.json
rdlp-probe extract --mode json-key 'm3u8'           --file response.json
```

Modes:
- `regex` — Rust regex; prints capture group 1 (or full match if no group). `--first` to print only the first.
- `css` — `scraper::Selector`; prints each match's outer HTML.
- `json` — RFC 6901 JSON pointer; prints the resolved value (string scalars are unwrapped — pipe-friendly).
- `json-key` — recursively walks the JSON and prints every value whose key equals the pattern.

### `record` — save a (request, response) pair as a JSON cassette

```bash
rdlp-probe record <url> -o crates/rdlp-extractor/tests/cassettes/<site>/page.json \
  -H 'Cookie: country=US' --note 'video page sample, 2026-04-25'
```

Cassette schema (stable):

```json
{
  "url": "...", "method": "GET",
  "request_headers": {...}, "request_body": null,
  "browser_emulation": "chrome",
  "recorded_at_unix": 1777134375,
  "note": "video page sample, 2026-04-25",
  "status": 200,
  "response_headers": {...},
  "response_body": "<full body as string>"
}
```

Cassettes are designed to be checked into `crates/rdlp-extractor/tests/cassettes/<site>/` and replayed by parser tests without network access.

## Recommended workflow when adding a new site

1. **Probe the site** — fetch a video page, look at headers and body.
   ```bash
   rdlp-probe fetch '<video-url>' -H 'Cookie: country=US' > page.html
   ```
2. **Record cassettes** for everything the extractor will need (page HTML, API responses, alternate URL shapes).
   ```bash
   rdlp-probe record '<video-url>' -o tests/cassettes/<site>/video.json --note 'main video page'
   rdlp-probe record '<api-url>' -X POST -d '...' -H ... -o tests/cassettes/<site>/formats.json
   ```
3. **Sketch the extraction** with `extract` to validate regexes and JSON paths against captured cassettes:
   ```bash
   rdlp-probe extract --mode json '/response_body' --file tests/cassettes/<site>/video.json \
     | rdlp-probe extract --mode regex 'data-streamkey="([^"]+)"' --first
   ```
4. **Decode any JS** with `eval` if the player URLs are obfuscated.
5. **Implement the extractor** in `crates/rdlp-extractor/src/extractors/<site>/`. Use the cassettes as the offline test fixtures (per `bug-fix-requires-failing-test.md`). Add a `#[ignore]`-marked live smoke test for CDN drift detection.
6. **Register** the extractor in `crates/rdlp-extractor/src/lib.rs::ExtractorRegistry::default`.

## Why a separate crate

- **Contributor entry point.** New extractor authors get a documented, supported tool — they don't need to read `BaseExtractor` source to figure out how to make a request that the production code would make.
- **Identical fingerprint.** Anything that requires browser TLS impersonation (Cloudflare-fronted sites, Akamai bot-management, etc.) works in `rdlp-probe` exactly because it shares `HttpClientFactory` with the live binary.
- **No bloat in the main binary.** `default-members` excludes `rdlp-probe`, so the release build of `rdlp` and `rdlp-desktop` is unaffected.

## What `rdlp-probe` does NOT do

- It does not solve JS challenges (Cloudflare Turnstile, reCAPTCHA). That requires a headless browser; out of scope.
- It does not run the rdlp orchestrator. Use `rdlp --dump-json <url>` for end-to-end extraction including format selection and rdlp's `Generic` fallback.
- It does not download formats. Use `rdlp` for that.
