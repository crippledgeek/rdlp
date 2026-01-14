# Contributing to rdlp

Thank you for your interest in contributing to rdlp! We welcome contributions from everyone, whether you're fixing bugs, adding features, improving documentation, or adding support for new sites.

## 🎯 Ways to Contribute

### 🐛 Report Bugs

Found a bug? Please [open an issue](https://github.com/yourusername/rdlp/issues/new) with:
- Clear description of the problem
- Steps to reproduce
- Expected vs actual behavior
- Your environment (OS, Rust version)
- Relevant logs (use `-v` flag for verbose output)

### 💡 Suggest Features

Have an idea? [Open an issue](https://github.com/yourusername/rdlp/issues/new) with:
- Clear description of the feature
- Use cases and benefits
- Possible implementation approach
- Willingness to implement it yourself

### 📝 Improve Documentation

Documentation improvements are always welcome:
- Fix typos or unclear explanations
- Add examples and use cases
- Improve API documentation
- Write guides or tutorials

### 🏗️ Add Site Extractors

One of the best ways to contribute is adding support for new sites. See [Adding a New Extractor](#adding-a-new-extractor) below.

### 🧪 Write Tests

More test coverage is always helpful:
- Unit tests for individual functions
- Integration tests for full workflows
- Edge case tests
- Performance benchmarks

## 🚀 Getting Started

### Prerequisites

- Rust 1.85+ (2024 Edition)
- Git
- Basic understanding of async Rust (tokio)

### Setup Development Environment

```bash
# Fork the repository on GitHub
# Then clone your fork
git clone https://github.com/YOUR_USERNAME/rdlp.git
cd rdlp

# Add upstream remote
git remote add upstream https://github.com/yourusername/rdlp.git

# Create a branch for your changes
git checkout -b feature/my-feature

# Build the project
cargo build

# Run tests
cargo test

# Run clippy
cargo clippy

# Format code
cargo fmt
```

## 📋 Development Workflow

### 1. Pick an Issue

- Browse [open issues](https://github.com/yourusername/rdlp/issues)
- Look for issues labeled `good first issue` if you're new
- Comment on the issue to let others know you're working on it

### 2. Create a Branch

```bash
git checkout -b feature/my-feature
# or
git checkout -b fix/bug-description
```

### 3. Make Changes

- Follow the [Code Style Guide](#code-style-guide)
- Write tests for new functionality
- Update documentation if needed
- Keep commits atomic and focused

### 4. Test Your Changes

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run clippy
cargo clippy -- -W clippy::all

# Format code
cargo fmt

# Build in release mode
cargo build --release
```

### 5. Commit Your Changes

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```bash
git add .
git commit -m "feat: add YouTube extractor"
# or
git commit -m "fix: handle 404 errors in HTTP downloader"
# or
git commit -m "docs: improve installation instructions"
```

**Commit Types:**
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation only
- `test:` - Adding or updating tests
- `refactor:` - Code change that neither fixes a bug nor adds a feature
- `perf:` - Performance improvement
- `chore:` - Maintenance tasks

### 6. Push and Create Pull Request

```bash
git push origin feature/my-feature
```

Then open a Pull Request on GitHub with:
- Clear title describing the change
- Description of what changed and why
- Reference to related issues (`Fixes #123`)
- Screenshots/examples if applicable

### 7. Code Review

- Address reviewer feedback
- Push additional commits if needed
- Once approved, a maintainer will merge your PR

## 🏗️ Adding a New Extractor

**⚖️ Legal Requirements**: Before adding an extractor, verify the site's Terms of Service:

✅ **Sites we support:**
- Explicitly allow downloading (e.g., Archive.org, Vimeo with privacy settings)
- Don't prohibit downloading in their ToS
- Designed for content distribution (creator platforms)
- Educational/archival purposes explicitly allowed

❌ **Sites we avoid:**
- Explicit ToS prohibitions against downloading
- Major streaming platforms with DRM
- Sites with technical anti-download measures
- Commercial services with subscription-only content

**User responsibility**: Contributors and users must ensure compliance with applicable laws and site ToS.

### Template

Use `crates/rdlp-extractor/src/extractors/tnaflix.rs` as a template:

```rust
use async_trait::async_trait;
use rdlp_core::{ExtractionContext, Format, InfoDict, InfoExtractor, Result, RdlpError};
use regex::Regex;
use scraper::{Html, Selector};

pub struct MyExtractor {
    name: String,
    url_pattern: Regex,
}

impl MyExtractor {
    pub fn new() -> Self {
        Self {
            name: "MySite".to_string(),
            url_pattern: Regex::new(
                r"https?://(?:www\.)?mysite\.com/watch\?v=([a-zA-Z0-9_-]+)"
            ).expect("Valid URL pattern"),
        }
    }
}

#[async_trait]
impl InfoExtractor for MyExtractor {
    fn name(&self) -> &str {
        &self.name
    }

    fn valid_url(&self) -> &Regex {
        &self.url_pattern
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // 1. Fetch webpage
        let response = ctx.http_client.get(url).send().await?;
        let html_text = response.text().await?;

        // 2. Parse HTML
        let document = Html::parse_document(&html_text);

        // 3. Extract metadata
        let title = extract_title(&document)?;
        let video_id = extract_id(url)?;

        // 4. Extract video URLs and build formats
        let formats = extract_formats(&document, ctx).await?;

        // 5. Build InfoDict
        let mut info = InfoDict::new(video_id, title, self.name.clone(), url.to_string());
        info.formats = formats;

        Ok(info)
    }

    fn priority(&self) -> i32 {
        0
    }
}

// Helper functions
fn extract_title(document: &Html) -> Result<String> {
    let selector = Selector::parse("title").unwrap();
    document.select(&selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .ok_or_else(|| RdlpError::Extraction("No title found".to_string()))
}

fn extract_id(url: &str) -> Result<String> {
    // Extract video ID from URL
    todo!()
}

async fn extract_formats(document: &Html, ctx: &ExtractionContext) -> Result<Vec<Format>> {
    // Extract video URLs and build formats
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_pattern() {
        let extractor = MyExtractor::new();
        assert!(extractor.suitable("https://www.mysite.com/watch?v=abc123"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=abc123"));
    }
}
```

### Register Your Extractor

In `crates/rdlp-extractor/src/lib.rs`:

```rust
mod extractors;

pub use extractors::mysite::MyExtractor;

// In registry.rs
impl ExtractorRegistry {
    pub fn new() -> Self {
        let mut extractors: Vec<Arc<dyn InfoExtractor>> = Vec::new();
        extractors.push(Arc::new(MyExtractor::new()));
        // ... other extractors
        Self { extractors }
    }
}
```

### Testing Your Extractor

```bash
# Run extractor tests
cargo test -p rdlp-extractor

# Test with actual URL
cargo run -- "https://www.mysite.com/watch?v=abc123"

# Test with verbose output
cargo run -- -v "https://www.mysite.com/watch?v=abc123"
```

## 📐 Code Style Guide

### General Principles

- **Readability First**: Code is read more than written
- **DRY**: Don't Repeat Yourself
- **KISS**: Keep It Simple, Stupid
- **YAGNI**: You Aren't Gonna Need It

### Rust Style

Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/):

- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Prefer `?` operator over `.unwrap()`
- Use descriptive variable names
- Add documentation comments for public APIs
- Use `Result<T>` for fallible operations
- Use `Option<T>` for optional values

### Error Handling

```rust
// Good: Propagate errors with context
let html = response.text().await
    .context("Failed to read response body")?;

// Bad: Panic
let html = response.text().await.unwrap();

// Bad: Ignore errors
let html = response.text().await.ok();
```

### Async Code

```rust
// Good: Use async/await
async fn download(&self, url: &str) -> Result<Vec<u8>> {
    let response = self.client.get(url).send().await?;
    let bytes = response.bytes().await?;
    Ok(bytes.to_vec())
}

// Bad: Blocking in async
async fn download(&self, url: &str) -> Result<Vec<u8>> {
    std::thread::sleep(Duration::from_secs(1)); // Don't block!
    // ...
}
```

### Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1048576), "1.0 MB");
    }

    #[tokio::test]
    async fn test_download() {
        let downloader = HttpDownloader::new();
        let result = downloader.download("https://example.com").await;
        assert!(result.is_ok());
    }
}
```

## 🔍 Pull Request Checklist

Before submitting your PR, ensure:

- [ ] Code compiles without warnings (`cargo build`)
- [ ] All tests pass (`cargo test`)
- [ ] Clippy is happy (`cargo clippy`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] New functionality has tests
- [ ] Documentation is updated
- [ ] Commit messages follow conventions
- [ ] PR description is clear and complete

## 🤝 Code Review Process

### What to Expect

1. **Automated Checks**: CI will run tests, clippy, and fmt
2. **Initial Review**: A maintainer will review within 48 hours
3. **Feedback**: You may be asked to make changes
4. **Approval**: Once approved, your PR will be merged

### Review Guidelines

- Be respectful and constructive
- Focus on the code, not the person
- Explain your reasoning
- Be open to feedback

## 📚 Additional Resources

### Documentation

- [IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) - Project architecture and roadmap
- [CLAUDE.md](CLAUDE.md) - Development guidelines for AI assistants
- [Rust Book](https://doc.rust-lang.org/book/) - Learn Rust
- [tokio Tutorial](https://tokio.rs/tokio/tutorial) - Async Rust

### Community

- [GitHub Discussions](https://github.com/yourusername/rdlp/discussions) - Ask questions
- [GitHub Issues](https://github.com/yourusername/rdlp/issues) - Report bugs
- [Matrix Chat](https://matrix.to/#/#rdlp:matrix.org) - Real-time chat (coming soon)

## 📝 License

By contributing to rdlp, you agree that your contributions will be dual-licensed under MIT OR Apache-2.0.

---

Thank you for contributing to rdlp! 🦀❤️
