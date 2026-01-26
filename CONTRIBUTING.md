# Contributing to rdlp

Thank you for your interest in contributing to rdlp! We welcome contributions from everyone, whether you're fixing bugs, adding features, improving documentation, or adding support for new sites.

## Ways to Contribute

### Report Bugs

Found a bug? Please open an issue with:
- Clear description of the problem
- Steps to reproduce
- Expected vs actual behavior
- Your environment (OS, Rust version)
- Relevant logs (use `-v` flag for verbose output)

### Suggest Features

Have an idea? Open an issue with:
- Clear description of the feature
- Use cases and benefits
- Possible implementation approach

### Add Site Extractors

One of the best ways to contribute is adding support for new sites. See [Adding a New Extractor](#adding-a-new-extractor) below.

### Write Tests

More test coverage is always helpful:
- Unit tests for individual functions
- Integration tests for full workflows
- Edge case tests

## Getting Started

### Prerequisites

- Rust 1.85+ (2024 Edition)
- Git
- FFmpeg (for post-processing features)
- Basic understanding of async Rust (tokio)

### Setup Development Environment

```bash
# Clone the repository
git clone https://github.com/YOUR_USERNAME/rdlp.git
cd rdlp

# Build the project
cargo build

# Run tests
cargo test

# Run clippy
cargo clippy

# Format code
cargo fmt
```

## Project Architecture

### Workspace Crates

| Crate | Purpose |
|-------|---------|
| `rdlp-core` | Core traits (`InfoExtractor`, `Downloader`, `PostProcessor`), error types |
| `rdlp-types` | Shared types (`InfoDict`, `Format`, `Config`, `Thumbnail`) |
| `rdlp-http` | Centralized HTTP client factory with connection pooling |
| `rdlp-security` | URL validation, SSRF protection, input sanitization |
| `rdlp-extractor` | Site extractors with base extractor framework |
| `rdlp-downloader` | HTTP + HLS downloaders with parallel chunking |
| `rdlp-jsinterp` | JavaScript interpretation for encrypted content |
| `rdlp-postprocess` | FFmpeg pipeline (merge, audio extract, metadata) |
| `rdlp-cookies` | Browser cookie extraction |
| `rdlp-plugin` | Plugin system architecture |
| `rdlp-cli` | CLI application and download orchestrator |

### Three-Stage Pipeline

1. **Extract**: URL -> metadata -> format list (`InfoExtractor` trait)
2. **Download**: Select format -> parallel chunks/HLS segments -> file (`Downloader` trait)
3. **Post-process**: FFmpeg transforms -> cleanup (`PostProcessor` trait)

### Extractor Architecture

Extractors follow a three-tier architecture:

```
Tier 1: BaseExtractor          (common utilities for ALL extractors)
Tier 2: TnaFlixNetworkBase     (shared logic for site families)
Tier 3: Site Extractors        (individual site implementations)
```

**Currently supported extractors:**
- TNAFlix (via `TNAFlixExtractor`)
- EMPFlix (via `TNAFlixExtractor`)
- MovieFap (via `TNAFlixExtractor`)
- RedTube (`RedTubeExtractor`)
- PornHub (`PornHubExtractor`) - includes playlist support

## Development Workflow

### 1. Create a Branch

```bash
git checkout -b feature/my-feature
# or
git checkout -b fix/bug-description
```

### 2. Make Changes

- Follow the [Code Style Guide](#code-style-guide)
- Write tests for new functionality
- Keep commits atomic and focused

### 3. Test Your Changes

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p rdlp-extractor

# Run tests with output
cargo test -- --nocapture

# Run clippy
cargo clippy -- -W clippy::all

# Format code
cargo fmt

# Build release
cargo build --release
```

### 4. Commit Your Changes

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```bash
git commit -m "feat(extractor): add YouTube extractor"
git commit -m "fix(downloader): handle 404 errors in HTTP downloader"
git commit -m "docs: improve installation instructions"
```

**Commit types:** `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`

**Scope examples:** `extractor`, `downloader`, `cli`, `core`, `postprocess`

## Adding a New Extractor

### Legal Requirements

Before adding an extractor, verify the site's Terms of Service. Contributors and users must ensure compliance with applicable laws and site ToS.

### Step 1: Create the Extractor Module

Create a new directory under `crates/rdlp-extractor/src/extractors/`:

```
extractors/mysite/
  mod.rs        # Main extractor implementation
  patterns.rs   # URL regex patterns
  formats.rs    # Format extraction logic (optional)
```

### Step 2: Define URL Patterns

In `patterns.rs`:

```rust
use once_cell::sync::Lazy;
use regex::Regex;

/// URL pattern for MySite videos
pub static MYSITE_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?mysite\.com/video/(?P<id>[a-zA-Z0-9]+)")
        .expect("Valid URL pattern")
});

/// Check if a URL is suitable for this extractor
pub fn is_suitable(url: &str) -> bool {
    MYSITE_URL_PATTERN.is_match(url)
}

/// Extract video ID from URL
pub fn extract_video_id(url: &str) -> Option<String> {
    MYSITE_URL_PATTERN
        .captures(url)
        .and_then(|caps| caps.name("id"))
        .map(|m| m.as_str().to_string())
}
```

### Step 3: Implement the Extractor

In `mod.rs`:

```rust
mod patterns;

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, InfoDict, InfoExtractor, Result, RdlpError};
use scraper::Html;

use crate::base::common::BaseExtractor;

pub use patterns::MYSITE_URL_PATTERN;

pub struct MySiteExtractor;

impl MySiteExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MySiteExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for MySiteExtractor {
    fn name(&self) -> &str {
        "MySite"
    }

    fn valid_url(&self) -> &regex::Regex {
        &MYSITE_URL_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // 1. Fetch webpage using BaseExtractor
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // 2. Extract video ID
        let video_id = patterns::extract_video_id(url)
            .ok_or_else(|| RdlpError::Extraction(
                format!("Could not extract video ID: {url}")
            ))?;

        // 3. Parse HTML and extract metadata
        //    NOTE: Html is not Send, so scope it in a block before any .await
        let (title, description, thumbnail) = {
            let html = Html::parse_document(&webpage);
            (
                extract_title(&html),
                extract_description(&html),
                extract_thumbnail(&html),
            )
        }; // html dropped here before await

        let title = title.ok_or_else(|| {
            RdlpError::Extraction("Could not find video title".to_string())
        })?;

        // 4. Extract formats
        let formats = extract_formats(&webpage, ctx).await?;

        if formats.is_empty() {
            return Err(RdlpError::Extraction(
                format!("No video formats found for URL: {url}")
            ));
        }

        // 5. Build InfoDict
        let mut info = InfoDict::new(
            video_id, title, self.name().to_string(), url.to_string()
        );
        info.description = description;
        info.thumbnail = thumbnail;
        info.formats = formats;

        Ok(info)
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        0
    }
}
```

**Important notes:**

- `Html` (from scraper) is **not `Send`** - always scope it in a block and drop it before any `.await` call
- Use `BaseExtractor::fetch_webpage()` for HTTP requests (handles errors and logging)
- All metadata fields on `InfoDict` are optional except `id`, `title`, `extractor`, and `webpage_url`
- Use `once_cell::sync::Lazy` for static regex patterns

### Step 4: Register the Extractor

In `crates/rdlp-extractor/src/extractors/mod.rs`, add your module:

```rust
pub mod mysite;
```

In `crates/rdlp-extractor/src/lib.rs`, register it:

```rust
pub use extractors::mysite::MySiteExtractor;

// In ExtractorRegistry::new():
registry.register(Arc::new(MySiteExtractor::new()));
```

### Step 5: Add Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = MySiteExtractor::new();
        assert_eq!(extractor.name(), "MySite");
    }

    #[test]
    fn test_suitable_urls() {
        let extractor = MySiteExtractor::new();

        assert!(extractor.suitable("https://www.mysite.com/video/abc123"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }
}
```

### Step 6: Test Manually

```bash
# Run extractor tests
cargo test -p rdlp-extractor

# Test with actual URL
cargo run -- "https://www.mysite.com/video/abc123"

# Test with verbose output
cargo run -- -v "https://www.mysite.com/video/abc123"
```

### JSON-LD Metadata

Many sites embed structured metadata in JSON-LD `<script>` tags. The `json_ld` module in `base/tnaflix_network/` provides parsers for `VideoObject` schema. If your target site uses JSON-LD, consider reusing or extending these parsers for:

- Title and description
- Upload date and duration (ISO 8601)
- Author/uploader (string or object format)
- Thumbnails (single or array)
- View/like counts (interaction statistics)
- Tags and categories

### Playlist Support

To support playlists, override `extract_playlist` in your `InfoExtractor` implementation:

```rust
async fn extract_playlist(
    &self,
    url: &str,
    ctx: &ExtractionContext,
) -> Result<Vec<InfoDict>> {
    if !is_playlist_url(url) {
        // Single video - delegate to extract()
        return Ok(vec![self.extract(url, ctx).await?]);
    }

    // Fetch playlist page, extract video URLs, extract each
    let mut results = Vec::new();
    for video_url in video_urls {
        results.push(self.extract(&video_url, ctx).await?);
    }
    Ok(results)
}
```

### HLS Format Detection

If the site serves HLS streams (`.m3u8`), use the `hls::detect_format_sizes` helper to probe segment counts and estimate file sizes:

```rust
use crate::hls::detect_format_sizes;

let formats_with_size = detect_format_sizes(formats, ctx, self.name()).await;
```

## InfoDict Fields

The `InfoDict` struct holds all extracted metadata. Required fields are set via `InfoDict::new()`. All other fields are optional and should be populated when the data is available:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | **Required.** Video ID |
| `title` | `String` | **Required.** Video title |
| `extractor` | `String` | **Required.** Extractor name |
| `webpage_url` | `String` | **Required.** Source URL |
| `formats` | `Vec<Format>` | Available download formats |
| `description` | `Option<String>` | Video description |
| `thumbnail` | `Option<String>` | Primary thumbnail URL |
| `thumbnails` | `Option<Vec<Thumbnail>>` | All available thumbnails |
| `uploader` | `Option<String>` | Uploader display name |
| `uploader_id` | `Option<String>` | Uploader identifier |
| `uploader_url` | `Option<String>` | Uploader profile URL |
| `channel` | `Option<String>` | Channel name |
| `channel_id` | `Option<String>` | Channel identifier |
| `channel_url` | `Option<String>` | Channel URL |
| `duration` | `Option<f64>` | Duration in seconds |
| `upload_date` | `Option<String>` | Upload date (YYYYMMDD format) |
| `view_count` | `Option<u64>` | View count |
| `like_count` | `Option<u64>` | Like count |
| `dislike_count` | `Option<u64>` | Dislike count |
| `average_rating` | `Option<f64>` | Average rating (0-100) |
| `age_limit` | `Option<u32>` | Age restriction |
| `tags` | `Option<Vec<String>>` | Tags/keywords |
| `categories` | `Option<Vec<String>>` | Content categories |
| `is_live` | `Option<bool>` | Whether the stream is live |
| `chapters` | `Option<Vec<Chapter>>` | Video chapters |

## Code Style Guide

### General Principles

- **Readability First**: Code is read more than written
- **DRY**: Don't Repeat Yourself
- **KISS**: Keep It Simple
- **YAGNI**: You Aren't Gonna Need It

### Rust Style

Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/):

- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Prefer `?` operator over `.unwrap()`
- Use descriptive variable names
- Add doc comments for public APIs
- Use `Result<T>` for fallible operations

### Error Handling

```rust
// Good: Propagate with context
let html = response.text().await
    .map_err(|e| RdlpError::Extraction(format!("Failed to read response: {e}")))?;

// Bad: Panic
let html = response.text().await.unwrap();
```

### Async Code

```rust
// Good: Non-blocking
async fn download(&self, url: &str) -> Result<Vec<u8>> {
    let response = self.client.get(url).send().await?;
    let bytes = response.bytes().await?;
    Ok(bytes.to_vec())
}

// Bad: Blocking in async context
async fn download(&self, url: &str) -> Result<Vec<u8>> {
    std::thread::sleep(Duration::from_secs(1)); // Never block!
    // ...
}
```

### Html and Send Safety

The `scraper::Html` type is not `Send`. Always scope it in a block before any `.await`:

```rust
// Good: Html dropped before await
let title = {
    let html = Html::parse_document(&webpage);
    extract_title(&html)
}; // html dropped here
let formats = fetch_formats(ctx).await?; // safe to await

// Bad: Html lives across await
let html = Html::parse_document(&webpage);
let title = extract_title(&html);
let formats = fetch_formats(ctx).await?; // compile error!
```

## Pull Request Checklist

Before submitting your PR, ensure:

- [ ] Code compiles without warnings (`cargo build`)
- [ ] All tests pass (`cargo test`)
- [ ] Clippy is happy (`cargo clippy`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] New functionality has tests
- [ ] Commit messages follow conventional commits

## License

By contributing to rdlp, you agree that your contributions will be licensed under the MIT License.
