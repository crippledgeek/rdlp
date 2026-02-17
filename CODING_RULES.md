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

## Project Structure

```
rdlp/
├── crates/
│   ├── rdlp-core/           # Traits, types, errors (foundation)
│   ├── rdlp-types/          # Pure domain types (no I/O)
│   ├── rdlp-http/           # HTTP client factory
│   ├── rdlp-security/       # URL validation, SSRF protection
│   ├── rdlp-extractor/      # Site-specific extractors
│   ├── rdlp-downloader/     # HTTP + HLS downloaders
│   ├── rdlp-postprocess/    # FFmpeg pipeline
│   ├── rdlp-cookies/        # Browser cookie extraction
│   ├── rdlp-jsinterp/       # JavaScript interpreter
│   ├── rdlp-plugin/         # Plugin system
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
