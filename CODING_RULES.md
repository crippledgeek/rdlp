# rdlp - Coding Standards

## Overview

This document establishes the core coding standards for the rdlp video downloader platform. All contributors must follow these guidelines to ensure consistency, maintainability, and quality across the codebase.

## Technology Stack

- **Rust 2024 Edition** - Primary programming language
- **Tokio** - Async runtime (multi-threaded)
- **Reqwest** - HTTP client with streaming support
- **Serde** - Serialization/deserialization
- **Clap** - CLI argument parsing
- **FFmpeg** - Post-processing (external)
- **Cargo** - Build and dependency management

## Code Style

### General Conventions
- **Indentation**: 4 spaces (no tabs)
- **Line Length**: Maximum 100 characters
- **Encoding**: UTF-8
- **Line Endings**: LF (Unix-style)
- **Formatting**: Run `cargo fmt` before committing

### Naming Conventions
- **Crates**: `rdlp-{domain}` (lowercase, hyphen-separated)
- **Modules**: `snake_case` (e.g., `hls_state`, `tnaflix_network`)
- **Types/Traits**: `PascalCase` (e.g., `InfoExtractor`, `DownloadStats`)
- **Functions/Methods**: `snake_case` (e.g., `extract_info`, `download_to_file`)
- **Variables**: `snake_case` (e.g., `segment_count`, `playlist_url`)
- **Constants**: `UPPER_SNAKE_CASE` (e.g., `MAX_SEGMENTS`, `DEFAULT_TIMEOUT`)
- **Test Functions**: `test_` prefix (e.g., `test_normalize_url`)

### Rust Best Practices
- Use **thiserror** for library errors, **anyhow** for application errors
- Prefer **constructor functions** (`new()`, `with_*()`) over public fields
- Use **builder pattern** for complex configuration
- Leverage **Rust 2024 features**: async traits, let chains
- Document public APIs with **rustdoc** (`///` comments)
- Add `#![warn(missing_docs)]` to all crates
- Prefer `impl Into<T>` for flexible function parameters
- Use **typed enums** for fixed-set values (never raw strings for container formats, protocols, etc.)
- Derive `PartialEq` (and `Eq` where no floats) on all value types for testability
- Prefer `eq_ignore_ascii_case()` over `to_lowercase()` comparisons (avoids allocation)

### Async Patterns
- Use `async_trait` for async trait methods
- Prefer `tokio::select!` for cancellation handling
- Use `Arc<AtomicU64>` for shared progress counters
- Apply `buffer_unordered` for bounded parallelism

### `tokio::fs::File` — Always flush before early return (Mandatory)

`tokio::fs::File`'s write methods (`write`, `write_all`, `write_buf`, `write_vectored`) schedule the actual I/O on a `spawn_blocking` task and **return BEFORE the kernel write completes**. Dropping the file synchronously (function return, `?` short-circuit) abandons the in-flight handle and previously-buffered bytes can disappear from disk. Documented at [tokio::fs](https://docs.rs/tokio/latest/tokio/fs/index.html): *"calls to `write` will return before the write has finished; `flush` will wait for the write to finish."*

`tokio::io::BufWriter` adds a second buffering layer (app-side buffer) with the same hazard: *"When the BufWriter is dropped, the contents of its buffer will be discarded."* No async Drop in Rust — neither layer auto-flushes.

**Rule:** Any function that owns a `tokio::fs::File` (directly or wrapped in `BufWriter`) and has `?` or `return Err(...)` paths MUST call `writer.flush().await.ok()` before each early return. The flush is best-effort — use `.ok()` so the original error message is preserved as the user-visible failure. The post-loop success-path flush MUST propagate its error normally (`flush().await.map_err(...)?`), since a flush failure on the success path is the primary failure.

**Canonical pattern** (mirrors `crates/rdlp-downloader/src/fragments.rs::download_pre_resolved_fragments`):

```rust
while let Some(item) = stream.next().await {
    // Replace `let v = item?;` with explicit match so we can flush on Err.
    let bytes = match item {
        Ok(v) => v,
        Err(e) => {
            out_file.flush().await.ok();
            return Err(e);
        }
    };

    // Replace `out_file.write_all(&bytes).await.map_err(...)?` likewise.
    if let Err(e) = out_file.write_all(&bytes).await {
        out_file.flush().await.ok();
        return Err(rdlp_core::RdlpError::Download {
            message: format!("write fragment: {e}"),
            url: Some(output.display().to_string()),
        });
    }
}

// Success-path flush at end of loop — propagate normally:
out_file
    .flush()
    .await
    .map_err(|e| rdlp_core::RdlpError::Download {
        message: format!("flush output: {e}"),
        url: Some(output.display().to_string()),
    })?;
```

**Why this is a hand-checked rule:** No clippy lint as of 2026-05 catches missing-flush-before-drop on async writers. `cargo-careful` doesn't catch it either (semantic loss, not UB). `scopeguard::defer!` runs synchronous closures and cannot `.await`, so RAII flush is not a safe alternative. Manual code review and this rule are the only gates.

**Reviewers MUST reject** any PR that introduces a new `tokio::fs::File` (or `BufWriter<tokio::fs::File>`) write loop with an Err short-circuit that lacks a flush before the early return.

**Atomic writes** via `tokio::fs::write(path, body)` (single call) and `tokio::fs::write_all_buf` on a fully-collected buffer are exempt — they have no intermediate drop point. Sync `std::fs::File` is also exempt — `std::io::BufWriter::Drop` attempts a sync flush (silently swallows any error, but data isn't lost in normal cases).

## Naming Standards — Methods

Authority chain: Rust API Guidelines (rust-lang/api-guidelines), Rust RFC 430, Robert C. Martin's *Clean Code* Ch. 2-3 (where it doesn't conflict with idiomatic Rust). Validated against axum and bevy production codebases.

| Rule | Statement | Example |
|---|---|---|
| M1 | No `get_` prefix on accessors. Use the bare noun: `filesize()` not `get_filesize()`. Exception: generic key-value lookups (`KvStore::get(key)`); also `StoreLimits::get_blocking` (K/V-store lookup in rdlp-plugin). | `Format::filesize()`, not `Format::get_filesize()` |
| M2 | `fetch_*` for HTTP/network I/O; `load_*` for local I/O (file, browser profile, config disk); `read_*` for byte-level reads on open handles. | `fetch_webpage(url)`, `load_cookies(path)`, `read_capped(reader, n)` |
| M3 | `extract_*` takes a parsed document (`Html`, `Value`, `&[u8]` with known structure); `parse_*` takes raw `&str` or `&[u8]`. | `extract_title(html: &Html)`, `parse_duration(s: &str)` |
| M4 | `to_*` prefix only for pure value conversions. Use `write_*` or `save_*` for file-writing side-effectful operations. | `Config::write_to_file(path)`, not `Config::to_toml_file(path)` |
| M5 | No `_async` suffix. `async fn` is the discriminator. Use `blocking_` prefix for blocking siblings (matches tokio convention). | `pub async fn fetch(url)`, `pub fn blocking_fetch(url)` |
| M6 | `try_` prefix for fallible constructors and fallible operations with infallible siblings. Do not use `try_` when the function always returns `Result` and there is no infallible counterpart. | `try_new(args) -> Result<Self>`, but `parse(s) -> Result<Self>` (always fallible — no prefix) |
| M7 | Search-page private helpers: `fetch_search_page()` for single-strategy extractors; `fetch_api_search_page()` / `fetch_html_search_page()` for dual-strategy. Drop vacuous `_single_` qualifier. | `tnaflix::fetch_search_page` (single strategy); `pornhub::fetch_api_search_page` + `pornhub::fetch_html_search_page` (dual) |
| M8 | Drop `_info` suffix when the return type already names the concept (`extract_video() -> InfoDict`). Exception: `_info` is part of the returned struct name (`EpisodeInfo`, `HlsInfo`, `MediaInfo`). | `extract_video() -> InfoDict`, not `extract_video_info() -> InfoDict` |

Special-case exceptions are documented inline in code with a comment referencing this rule set (e.g. `codec_threading_info` returns `(i32, i32)` tuple, not a struct, but `_info` names the concept of "threading information"; `const fn` unsafe FFI helper, rename deferred).

## Naming Standards — Parameters

Authority chain: Robert C. Martin's *Clean Code* Ch. 3 (Function Arguments), Rust API Guidelines (C-CASE, C-GENERIC, C-WORD-ORDER), tokio + stdlib precedents (`tokio::process::Command::kill_on_drop`, `std::fs::copy/write`).

| Rule | Statement | Example |
|---|---|---|
| P1 | No bare `bool` parameter in public non-builder API. Use a two-variant enum, split into named methods, or — for builder-style methods only — accept `bool` because the method name carries the semantics. | Split: `pub async fn download(url)` and `pub async fn download_interactive(url)`, not `download(url, interactive: bool)` |
| P2 | Extract a params struct when a function reaches 5 args, especially with multiple positional `u64`s (swap-risk) or 2+ `bool`s. Threshold enforced by `clippy::too_many_arguments` at 5 (configured via `clippy.toml`). | `fn substitute(template: &str, rep_id: &str, vars: &DashTemplateVars) -> String`, not 5-positional-arg form |
| P3 | Use `_name: T` (not bare `_: T`) for unused parameters on documented public trait impls. The name appears in rustdoc, rust-analyzer inlay hints, and compiler error messages. Test-only `#[cfg(test)]` mocks may keep bare `_:`. | `fn confirm(&self, _request: ConfirmRequest) -> ConfirmResponse`, not `fn confirm(&self, _: ConfirmRequest)` |
| P4 | Spell out abbreviations (`cb` → `callback`, `forwarder`, or descriptive event-verb) on public methods. Internal short names (`buf`, `len`, `cx`, `f`, `n`) are acceptable where type and context make the role clear. | `pub fn set_log_forwarder(forwarder: Arc<dyn Fn(...)>)`, not `cb:` |
| P5 | Use `impl AsRef<Path>` / `impl Into<String>` for public path/string params (C-GENERIC); use `&Path` / `&str` for private helpers. Avoid `impl AsRef<str>` (uncommon in ecosystem). | Public: `fn from_toml_file(path: impl AsRef<Path>)`. Private: `fn merge_stream_path(base: &Path, ...)`. |
| P6 | Parameter ordering: source before destination (`from`/`to`); target before data (`path`/`contents`); callbacks and cancellation tokens last. | `fn download_format(format, path, progress, cancel)` — matches stdlib `fs::copy(from, to)` and `fs::write(path, contents)` |
| P7 | Generic type parameters use single uppercase letters (`T`, `U`, `K`, `V`, `E`, `R`). Value parameters use descriptive snake_case names. | `fn first<T>(items: &[T]) -> Option<&T>` |
| P8 | Builder-flag method signature: `fn with_<behavior>(self, <behavior>: bool) -> Self`. The method name supplies the semantics; the bare `bool` is acceptable. | `.with_adaptive(true)` — readable at the call site |

## Project Structure

```
rdlp/
├── crates/
│   ├── rdlp-types/          # Pure domain types (no I/O)
│   ├── rdlp-table/          # Column layout constants for format table
│   ├── rdlp-core/           # Traits, types, errors (foundation)
│   ├── rdlp-security/       # URL validation, SSRF protection
│   ├── rdlp-http/           # HTTP client factory
│   ├── rdlp-ratelimit/      # Async token-bucket rate limiter
│   ├── rdlp-crypto/         # PRNG-based URL decryption
│   ├── rdlp-extractor/      # Site-specific extractors
│   ├── rdlp-downloader/     # HTTP + HLS downloaders
│   ├── rdlp-ffmpeg/         # FFmpeg library bindings wrapper
│   ├── rdlp-postprocess/    # FFmpeg pipeline + mp4ameta
│   ├── rdlp-cookies/        # Browser cookie extraction
│   ├── rdlp-jsinterp/       # JavaScript interpreter
│   ├── rdlp-plugin/         # Plugin system
│   ├── rdlp-api/            # Frontend-agnostic API layer
│   ├── rdlp-desktop/        # Tauri v2 desktop GUI
│   └── rdlp-cli/            # CLI application
├── docs/                    # Documentation (non-root)
├── tests/                   # Integration tests
├── Cargo.toml               # Workspace manifest
├── README.md                # Project overview
├── BUILDING.md              # Building from source
├── CLAUDE.md                # AI assistant guidance
├── CODING_RULES.md          # This file
└── CONTRIBUTING.md          # Contribution guidelines
```

## Crate Structure

Each crate follows a standard structure:

```
rdlp-{crate}/
├── src/
│   ├── lib.rs               # Crate root, exports, docs
│   ├── error.rs             # Error types (if needed)
│   ├── {module}.rs          # Feature modules
│   └── {module}/
│       ├── mod.rs           # Module root
│       └── {submodule}.rs   # Submodules
├── tests/                   # Integration tests
└── Cargo.toml               # Crate manifest
```

### Module Guidelines
- **Max 500 LOC per file** - Split larger files into submodules
- **Single responsibility** - Each module has one clear purpose
- **Minimal pub exports** - Use `pub(crate)` for internal APIs
- **Document module purpose** - Add `//!` docs at module top

## Testing Standards

### Test Organization
- Unit tests in same file: `#[cfg(test)] mod tests { ... }`
- Integration tests in `tests/` directory
- Use `#[tokio::test]` for async tests
- Use `proptest` for property-based testing where applicable

### Naming and Style
```rust
#[test]
fn test_normalize_url_strips_query_params() { ... }

#[tokio::test]
async fn test_download_resumes_from_partial() { ... }
```

### Coverage Requirements
- Test happy paths, error cases, and edge conditions
- Mock external services with `mockito`
- Use `tempfile` for filesystem tests
- Run `cargo test` before committing
- Run `cargo clippy` with zero warnings

### Test Commands
```bash
cargo test                           # All tests
cargo test --package rdlp-downloader # Single crate
cargo test test_name                 # Specific test
cargo test -- --nocapture            # Show println output
```

## Type Safety

### Strongly-Typed Enums
Use the following enums from `rdlp-types` instead of raw strings. All are `Serialize`/`Deserialize` compatible and implement `Display`/`FromStr`.

| Enum | Use For | Variants |
|------|---------|----------|
| `ContainerFormat` | Container/remux/recode config | Mp4, Mkv, WebM, Mov, Ts, Flv, Avi, Ogg, M4a |
| `AudioFormat` | Audio extraction format | Mp3, Aac, M4a, Opus, Vorbis, Flac, Alac, Wav |
| `BrowserType` | Cookie browser selection | Chrome, Firefox |
| `DownloadProtocol` | Format download protocol | Http, Https, M3u8, M3u8Native, HttpDashSegments, Other(String) |
| `SubtitleFormat` | Subtitle format config | Srt, Vtt, Ass, Ssa, Lrc |

```rust
// Correct — use typed enums
config.remux_container = Some(ContainerFormat::Mp4);
config.audio_format = Some(AudioFormat::Mp3);
let format = Format::new("hd", url, "mp4", DownloadProtocol::Https);

// Wrong — raw strings for fixed-set values
config.remux_container = Some("mp4".to_string());  // DON'T
```

Parse CLI string inputs at the boundary:
```rust
let container: ContainerFormat = user_input.parse().map_err(|e| anyhow::anyhow!(e))?;
```

### Structured Errors
- `Config::validate()` returns `Result<(), ConfigValidationError>` (not `String`)
- `RdlpError::Http { status, reason }` for HTTP errors (not `Network` with status in string)
- `is_retryable_error()` matches on `RdlpError::Http { status, .. }` for 429/5xx

### Derive Guidelines
- Add `PartialEq` to all value types (enables `assert_eq!` in tests)
- Add `Eq` where there are no `f64` fields
- Add `Copy` to small enums without heap data

## Error Handling

### Error Types
- **Library crates**: Define errors with `thiserror`
- **Application (CLI)**: Use `anyhow::Result<T>`
- **User cancellation**: Return `Ok(None)`, not an error

### Error Pattern
```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
```

### Context Addition
```rust
use anyhow::Context;

let data = fetch_url(&url)
    .await
    .context("Failed to fetch playlist")?;
```

### URL Redaction — Controls (tiered)

URLs may carry credentials (signed CDN tokens, OAuth, AWS SigV4) in userinfo/query.
Three complementary controls, in priority tiers, keep them out of logs and error
messages (CWE-532 / OWASP "redact at source"). **Each catches what the others cannot.**

1. **The type system is the PRIMARY guard.** A URL that can reach operator-visible
   output (an error `Display`, `user_message()`, a `format!`, a log field) must be
   stored as `rdlp_redact::RedactedUrlBuf` (or borrowed `RedactedUrl<'_>`), whose
   `Display`/`Debug` redact via `redact_str`. Then `#[error("…{url}")]` and
   `format!("…{url}")` auto-redact — the raw value can only escape via an explicit
   `.expose()`. This is "secrets-as-types": redaction is compiler-enforced, not a
   convention a future call site can forget. Mirror the precedents
   `RdlpError::Extraction.url: Option<RedactedUrlBuf>` and
   `RdlpApiError::UnsupportedUrl.url: RedactedUrlBuf` (#427). **Never** store a raw
   `String` URL in an error field that a Display/format interpolates, and never
   store a "pre-redacted `String`" (the type must carry the guarantee).

2. **Semgrep gate is the SECONDARY backstop** (`scripts/check-log-redaction.sh`,
   rule `scripts/semgrep/log-url-redaction.yml`; Semgrep `generic` mode — Rust mode
   cannot cross the `log` macro `;` kv/message separator). It flags any raw value
   passed to a `*url*` key in a `log` macro (`debug!/info!/warn!/error!/trace!`) —
   including the no-sigil `url = x` and two-kv forms the regex cannot reach — and is
   sanitizer-aware: `RedactedUrl::new(...)`, `RedactedUrlBuf`, `sanitize_for_logging(...)`,
   and their qualified `rdlp_redact::*` / `rdlp_security::*` forms are recognized
   sanitizers, so wrapped sites pass. This is the CWE-532-recommended control class
   (automated static analysis) for the residual log-field surface a redacting
   type cannot force (a value that must stay raw for I/O in the same function, e.g.
   a thumbnail URL used for the GET — #428).

3. **`scripts/check-url-redaction.sh` is the TERTIARY canary (defense-in-depth).**
   It greps for raw URL interpolation idioms in `rdlp-extractor`, `rdlp-postprocess`,
   and `rdlp-api`. **It deliberately does NOT gate error-`Display`/`format!` `{url}`
   sites in `rdlp-api`** — a grep cannot distinguish a redacted `{url}`
   (`RedactedUrlBuf`) from a raw one (`String`); both are the same token in source,
   and it would false-positive on the compliant `{source_url}` precedent. That
   surface is the type system's job (control 1). The grep's residual role is the
   surface the type cannot cover: a raw URL passed to a structured-kv log field
   (`url:? = some_string`) — i.e. an `.expose()`-then-log bypass.

When you add a new URL-bearing error variant or log a URL: pick the field type
(`RedactedUrlBuf`) first; the grep gate is the backstop, not the design.

## Code Reuse

### Extractor Format Building
All site extractors **must** use `BaseExtractor::build_format()` to construct `Format` structs. This centralises height/width calculation, format_note, codec defaults, and quality scoring.

```rust
// Correct — delegate to BaseExtractor, then add site-specific fields
let mut format = BaseExtractor::build_format(format_id, url, ext, height);
format.container = Some(container.to_owned()); // site-specific
format.tbr = extract_bitrate_from_url(&url);   // site-specific
```

```rust
// Wrong — duplicating Format construction in each extractor
let mut format = Format::new(id, url, ext, DownloadProtocol::Https);
format.height = Some(h);
format.width = Some(width_from_height(h));
format.vcodec = Some("h264".to_string());
// ...
```

### Shared Utilities
- Use `BaseExtractor` methods (`parse_quality_height`, `width_from_height`, `extract_meta_content`, `extract_element_text`, etc.) instead of defining local equivalents.
- Site-specific helpers belong in the extractor's own module but should compose on top of `BaseExtractor`, not replace it.
- When adding a new extractor, check `BaseExtractor` and `crate::utils` for existing helpers before writing new ones.

## File Cohesion & Function Length

**Function length (hard rule, enforced by clippy).** No function may exceed 150 code lines (blank and comment lines excluded). The `clippy::too_many_lines` lint enforces this workspace-wide. Per-function `#[allow(clippy::too_many_lines)]` is permitted with a `// reason:` comment naming the structural justification (long match arm, state machine, FFI dispatch, subcommand dispatch).

**File cohesion (soft, audit-driven).** Audit production files (excluding tests) at 800 LOC. Justify above 1200 LOC in the PR body. The test for cohesion is whether a new reader must scroll past unrelated concepts to understand one — not line count alone. Use `scripts/count-loc.sh` to compute non-test LOC for a file or directory.

**Struct growth.** When a struct accumulates 15+ methods or its `impl` block grows past ~600 LOC, extract sub-concerns into new types (helper struct, inner state, extension trait). Do not split a single `impl` block across files via `mod foo; impl Foo {...}` — that pattern is legal Rust but confuses tooling and is virtually absent in well-known crates.

**Test placement.** Default to inline `#[cfg(test)] mod tests {}` co-located with the code under test. When the inline test module exceeds ~300 LOC and dominates production readability, extract to a child-module sibling file via `#[cfg(test)] mod tests;` + `foo/tests.rs`. This is a per-file judgment; the child-module form preserves private-item access without `pub(crate)` escalation.

**Workspace empirical context (2026-05-22).** rdlp's production median is 248 LOC, p90 557 LOC — tighter than the Rust community median (300–500). The thresholds above are calibrated as tail-outlier hardening, not workspace-wide compression. See docs/superpowers/specs/2026-05-22-file-cohesion-policy-design.md for the research record. **Caveat**: workspace lints (including `too_many_lines`) only fire on crates with `[lints] workspace = true` in their `Cargo.toml` — currently 4 of 19 crates (rdlp-cli, rdlp-cookies, rdlp-extractor, rdlp-plugin-manifest). Broadening lint inheritance to additional crates requires upfront migration of their `unwrap_used`/`expect_used` posture, which is out of scope here.

## API Design

### Trait Design
```rust
#[async_trait]
pub trait Downloader: Send + Sync {
    fn protocol(&self) -> &str;
    fn supports(&self, url: &str) -> bool;
    async fn download_to_file(&self, url: &str, path: &Path) -> Result<DownloadStats>;
}
```

### Builder Pattern
```rust
let downloader = HttpDownloader::new()
    .with_concurrent_fragments(8)
    .with_buffer_size(2 * 1024 * 1024)
    .with_retry_config(RetryConfig::default());
```

### Progress Callbacks
```rust
pub trait ProgressCallback: Send + Sync {
    fn on_progress(&self, progress: &DownloadProgress);
    fn on_complete(&self, stats: &DownloadStats);
    fn on_error(&self, error: &str);
}
```

## Performance Standards

### Download Optimization
- **Chunk sizing**: Power-of-two (64KB - 8MB)
- **Parallelism**: 4-8 concurrent connections
- **Buffering**: 2MB I/O buffers
- **Connection pooling**: 10 connections per host

### Memory Management
- Use `BufWriter` for file writes
- Stream large responses (don't load into memory)
- Clean up temporary files on success and error
- Use `Arc` for shared state, not cloning data

## Git Workflow

### Branch Strategy
```bash
# Feature branches
feature/{description}

# Bug fixes
fix/{description}

# Examples
feature/hls-resume-support
fix/cdn-hostname-matching
```

### Commit Messages
- **Format**: `type(scope): description`
- **Types**: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`
- **Examples**:
  - `feat(downloader): add HLS resume support`
  - `fix(security): handle CDN hostname rotation`
  - `docs(readme): update installation instructions`

### Pre-Commit Checklist
Before committing, ensure:
- ✅ `cargo build` passes
- ✅ `cargo test` passes
- ✅ `cargo clippy` has zero warnings
- ✅ `cargo fmt` has been run
- ✅ No secrets or credentials in code
- ✅ Documentation updated if API changed

## Documentation Standards

### Code Documentation
```rust
/// Extract video metadata from a URL.
///
/// # Arguments
/// * `url` - The video page URL
/// * `ctx` - Extraction context with HTTP client
///
/// # Returns
/// * `Ok(InfoDict)` - Extracted metadata
/// * `Err(_)` - Extraction failed
///
/// # Example
/// ```no_run
/// let info = extractor.extract_info(url, &ctx).await?;
/// ```
pub async fn extract_info(&self, url: &str, ctx: &ExtractContext) -> Result<InfoDict>;
```

### Project Documentation
- **Root**: Only `README.md`, `BUILDING.md`, `CLAUDE.md`, `CODING_RULES.md`, `CONTRIBUTING.md`
- **All other docs**: In `docs/` subdirectory
- **Governance**: See `docs/documentation-standards.md` (not committed to git)
- **No auto-generated docs**: Keep docs intentional and maintained

## Security Standards

### URL Handling
- Validate URLs with `rdlp_security::validate_url_security()`
- Block private/internal hosts (SSRF protection)
- Sanitize URLs before logging (strip tokens)
- Use `extract_url_path()` for CDN-tolerant comparison

### Sensitive Data
- Never log tokens, passwords, or session IDs
- Use `sanitize_for_logging()` for URL logging
- Don't commit `.env` files or credentials
- Respect `max_url_length` limits

## Quick Reference

### Build Commands
```bash
cargo build --release        # Optimized build
cargo test                   # Run all tests
cargo clippy                 # Lint check
cargo fmt                    # Format code
cargo doc --open             # Generate docs
```

### Development Workflow
```bash
# Run in development
cargo run -- "https://example.com/video"

# Run with logging
RUST_LOG=debug cargo run -- -v "https://example.com/video"

# Run specific test
cargo test --package rdlp-downloader test_hls_resume

# Check before commit
cargo fmt && cargo clippy && cargo test
```

## Enforcement

- All code must pass `cargo build` and `cargo test`
- Zero `cargo clippy` warnings required
- Code must be formatted with `cargo fmt`
- Public APIs must have rustdoc documentation
- Follow branch naming and commit message conventions
- Complete pre-commit checklist before merging

---

**For legal guidelines on adding new extractors, see [CONTRIBUTING.md](CONTRIBUTING.md)**
