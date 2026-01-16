# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**rdlp** is a Rust project using edition 2024, focused on asynchronous web scraping and HTTP operations.

### Core Dependencies
- `tokio`: Async runtime with multi-threaded executor
- `reqwest`: HTTP client with native-tls and streaming support
- `scraper`: HTML parsing and CSS selector-based extraction
- `anyhow`: Error handling with context
- `async-trait`: Async trait support
- `url`: URL parsing and manipulation

## Development Commands

### Build and Run
```bash
# Build the project
cargo build

# Build with optimizations
cargo build --release

# Run the application
cargo run

# Run with release optimizations
cargo run --release
```

### Testing
```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run tests in single-threaded mode (useful for async tests)
cargo test -- --test-threads=1
```

### Code Quality
```bash
# Check code without building
cargo check

# Run clippy for lints
cargo clippy

# Run clippy with all lints
cargo clippy -- -W clippy::all

# Format code
cargo fmt

# Check formatting without changes
cargo fmt -- --check
```

## Architecture Notes

### Async Runtime
The project uses `tokio` as the async runtime with:
- `macros` feature: Enables `#[tokio::main]` and `#[tokio::test]` macros
- `rt-multi-thread` feature: Multi-threaded work-stealing scheduler
- `signal` feature: Unix/Windows signal handling (Ctrl+C interrupt support)
- `fs` feature: Async file I/O operations
- `io-util` feature: Async read/write utilities

### HTTP Client Configuration
`reqwest` is configured with:
- `native-tls` for TLS support (default OpenSSL disabled)
- `stream` feature for streaming response bodies

This configuration is optimal for web scraping workloads where streaming HTML content is common.

### Error Handling Pattern
Use `anyhow::Result<T>` for application-level error handling with context:
```rust
use anyhow::{Context, Result};

fn example() -> Result<()> {
    something()?
        .context("Failed to do something")?;
    Ok(())
}
```

### Web Scraping Pattern
Combine `reqwest` for HTTP and `scraper` for HTML parsing:
```rust
use scraper::{Html, Selector};

let html = reqwest::get(url).await?.text().await?;
let document = Html::parse_document(&html);
let selector = Selector::parse("css-selector").unwrap();
```

### User Cancellation Pattern
Use `Result<Option<T>>` for operations that users can cancel (not an error):
```rust
// Function that can be cancelled
async fn download(&self, url: &str, interactive: bool) -> Result<Option<PathBuf>> {
    let format = if interactive {
        match self.select_format_interactive(&formats)? {
            Some(format) => format,
            None => {
                println!("Selection cancelled by user");
                return Ok(None);  // Not an error!
            }
        }
    } else {
        // ... automatic selection
    };

    // ... do work
    Ok(Some(output_path))
}
```

### Signal Handling Pattern
Use `tokio::select!` to race async operations with Ctrl+C:
```rust
use tokio::signal;

let stats = tokio::select! {
    result = download_future => {
        result.context("Download failed")?
    }
    _ = signal::ctrl_c() => {
        println!("⏸️  Download interrupted by user");
        return Ok(None);  // Save state and return gracefully
    }
};
```

### Send Trait Considerations
When working with HTML parsing across await points, extract all data synchronously first:
```rust
// Extract all data from HTML before any async operations
let (title, description, metadata) = {
    let html = Html::parse_document(&webpage);

    // Extract everything synchronously (no .await calls)
    let title = extract_title(&html)?;
    let description = extract_description(&html)?;
    let metadata = extract_metadata(&html)?;

    (title, description, metadata)
}; // html is dropped here

// Now make async requests without holding Html reference
let formats = self.build_formats(metadata, ctx).await;
```

### Filesize Detection
Implement robust filesize detection with fallback strategies:
```rust
// Try HEAD request first
match http_client.head(&video_url).send().await {
    Ok(response) => {
        format.filesize = response.content_length();

        // Fallback to Range request if HEAD returns no size
        if format.filesize.is_none() || format.filesize == Some(0) {
            match http_client
                .get(&video_url)
                .header("Range", "bytes=0-0")
                .send()
                .await
            {
                Ok(range_response) => {
                    // Parse Content-Range: "bytes 0-0/123456"
                    if let Some(content_range) = range_response.headers().get("content-range") {
                        if let Ok(range_str) = content_range.to_str() {
                            if let Some(total) = range_str.split('/').nth(1) {
                                format.filesize = total.parse::<u64>().ok();
                            }
                        }
                    }
                }
                Err(_) => { /* continue without filesize */ }
            }
        }
    }
    Err(_) => { /* continue without filesize */ }
}
```

### Auto-Resume Pattern
Check for partial downloads and resume automatically:
```rust
let resume_from = if output_path.exists() {
    match tokio::fs::metadata(&output_path).await {
        Ok(metadata) => {
            let size = metadata.len();
            if size > 0 {
                println!("📋 Found partial download, resuming...");
                size
            } else { 0 }
        }
        Err(_) => 0,
    }
} else { 0 };

// Use download_with_resume if we have partial data
if resume_from > 0 {
    downloader.download_with_resume(&url, &path, resume_from, progress_callback).await?
} else {
    downloader.download_to_file(&url, &path, progress_callback).await?
}
```

## Performance Optimizations

### Multi-Layered Download Optimization
rdlp implements a comprehensive 6-layer optimization stack for maximum download speed:

#### 1. Parallel Chunk Downloads
- **Default**: 4 concurrent connections (configurable via `concurrent_fragments`)
- **Automatic activation**: Files > 10MB with Range request support
- **How it works**: File is split into equal chunks, downloaded in parallel, then merged
- **Performance**: 3-5x faster than sequential downloads
- **Smart resume**: Auto-switches to parallel mode if < 20% downloaded

```rust
config.concurrent_fragments = 8; // Use 8 parallel connections
```

#### 2. Multi-Threaded Tokio Runtime
- **Worker threads**: 2x CPU cores (capped at 32)
- **Optimized for I/O**: More threads = more concurrent network operations
- **Example**: 8-core CPU = 16 worker threads
- **Implementation**: `tokio::runtime::Builder::new_multi_thread()`

#### 3. Buffered I/O
- **Buffer size**: 2MB (250x larger than original 8KB)
- **Impact**: 50-100x fewer disk write syscalls
- **Memory efficient**: Constant memory usage regardless of file size
- **Implementation**: `BufWriter::with_capacity(2MB, file)`

#### 4. HTTP Client Optimizations
- **Connection pooling**: Keeps 10 connections alive per host
- **Pool timeout**: 90 seconds idle timeout
- **TCP keepalive**: 60-second intervals prevent connection drops
- **TCP_NODELAY**: Disables Nagle's algorithm for lower latency
- **Smart timeouts**: 30s connect, 60s idle (no total time limit)

#### 5. Intelligent Size Detection
- **Primary**: HEAD request for Content-Length
- **Fallback**: Range request parsing Content-Range header
- **Example**: `Content-Range: bytes 0-0/618618881` → extracts total size

#### 6. Real-Time Progress Tracking
- **Update interval**: 100ms
- **Shared atomic counter**: All chunks update progress in real-time
- **Async task**: Dedicated progress reporter running in parallel

### Performance Benchmarks
**Test case**: 590 MB video download from TNAFlix

| Optimization Level | Time | Speed | Improvement |
|-------------------|------|-------|-------------|
| Initial (8KB buffer, single thread) | 35+ min | ~360 KB/s | Baseline |
| + Buffered I/O (2MB) | ~15 min | ~650 KB/s | 2x faster |
| + Connection pooling | ~12 min | ~820 KB/s | 2.5x faster |
| + Parallel chunks (4 connections) | ~9 min | ~1.1 MB/s | **3x faster** |
| + Multi-threaded runtime | ~6-8 min | ~1.5 MB/s | **4-5x faster** |

**Note**: Actual speeds depend on server throttling, network conditions, and bandwidth.

## Project Architecture

### Workspace Structure
rdlp uses a workspace-based architecture with 8 separate crates:
- **rdlp-core**: Core traits, types, and errors (foundation for all other crates)
- **rdlp-extractor**: Extractor framework and site-specific extractors (TNAFlix, EMPFlix, MovieFap)
- **rdlp-downloader**: Download protocol implementations (HTTP, HLS, DASH)
- **rdlp-jsinterp**: JavaScript execution engine using boa
- **rdlp-postprocess**: Post-processing pipeline (FFmpeg integration)
- **rdlp-cookies**: Browser cookie extraction
- **rdlp-plugin**: Dynamic plugin loading system
- **rdlp-cli**: User-facing CLI application

### Core Traits
The architecture is built around these key traits from `rdlp-core`:
- **InfoExtractor**: Extract video metadata from URLs
- **Downloader**: Download content via various protocols
- **PostProcessor**: Transform downloaded files
- **JsEngine**: Execute JavaScript code
- **CookieJar**: Manage cookies for authentication

### Three-Stage Pipeline
1. **Extraction**: Parse URL → Extract metadata → Get format list
2. **Download**: Select format → Download fragments → Merge streams
3. **Post-Processing**: Convert formats → Embed metadata → Clean up

### Key Data Structures
- **InfoDict**: Central metadata structure flowing through pipeline
- **Format**: Video/audio format information
- **Config**: Application configuration

## Implementation Status

### Phase 1: Foundation ✓ COMPLETE
- [x] Workspace structure with 8 crates
- [x] Core traits (InfoExtractor, Downloader, PostProcessor, JsEngine)
- [x] InfoDict and Format structures
- [x] Error types using thiserror
- [x] Config structure with TOML/YAML support
- [x] All tests passing

### Phase 2: TNAFlix Support ✓ COMPLETE
- [x] HTTP downloader with streaming support
- [x] Progress tracking with indicatif
- [x] Resume capability (Range requests, auto-resume on restart)
- [x] Ctrl+C interrupt handling with progress save
- [x] Downloader registry
- [x] TNAFlix/EMPFlix/MovieFap extractors
- [x] XML config parsing
- [x] Metadata extraction (title, description, uploader, thumbnail)
- [x] Filesize detection via HEAD/Range requests with Content-Range fallback
- [x] Format selection (basic selector + interactive mode)
- [x] Interactive format selection with ESC cancellation
- [x] CLI orchestrator with graceful cancellation handling
- [x] Working end-to-end downloads
- [x] Code quality: Zero clippy warnings
- [x] **Performance Optimizations** (3-5x speed improvement):
  - [x] Parallel chunk downloads (4 concurrent connections by default)
  - [x] Multi-threaded tokio runtime (2x CPU cores for I/O workloads)
  - [x] Buffered I/O with 2MB buffers
  - [x] HTTP connection pooling (10 connections/host)
  - [x] TCP_NODELAY for lower latency
  - [x] Smart timeout configuration (60s idle, unlimited total time)
  - [x] Automatic parallel mode switching for resumed downloads < 20%

### Upcoming Development
**Note**: YouTube support is deferred due to explicit Terms of Service restrictions. Focus is on sites with permissive ToS.

- **Next**: HLS/DASH protocol support for adaptive streaming
- **Next**: Additional extractors (Vimeo, Dailymotion, Archive.org)
- **Next**: Enhanced format selection DSL parser
- **Future**: FFmpeg post-processing and format conversion
- **Future**: Browser cookie extraction for authenticated sites
- **Future**: Plugin system for community extractors
- **Future**: Polish and production release

See CONTRIBUTING.md for legal requirements when adding new site support.

## Usage Examples

### Download a video (optimized release build recommended)
```bash
# Use release build for maximum performance
cargo run --release -- "https://www.tnaflix.com/hd-videos/title/video123456"

# Or run the compiled binary directly
.\target\release\rdlp.exe "https://www.tnaflix.com/hd-videos/title/video123456"
```

### Interactive format selection
```bash
# Use -i flag to choose format interactively (press ESC to cancel)
cargo run --release -- -i "https://www.empflix.com/videos/title-123"
```

### Interrupt and resume downloads
```bash
# Press Ctrl+C during download to pause and save progress
# Run the same command again to automatically resume from where you left off
cargo run -- "https://www.moviefap.com/videos/abc123/title.html"
```

### List available extractors
```bash
cargo run -- --list-extractors
```

### Specify output directory
```bash
cargo run -- -o ./downloads "https://www.empflix.com/videos/title-123"
```

### Select specific format
```bash
cargo run -- -f best "https://www.moviefap.com/videos/abc123/title.html"
```

### Verbose mode (shows detailed extraction info)
```bash
cargo run --release -- -v "https://www.tnaflix.com/hd-videos/title/video123456"
```

### Parallel downloads (automatic for files > 10MB)
The downloader automatically uses parallel mode when:
- File size > 10 MB
- Server supports Range requests
- `concurrent_fragments` > 1 (default: 4)

You'll see output like:
```
📊 Download analysis:
   - Detected size from Range: 590 MB
🚀 Using parallel download mode (4 connections)
🚀 Starting 4 parallel downloads...
   Chunk 0: 0 MB - 147 MB (147 MB)
   Chunk 1: 147 MB - 295 MB (147 MB)
   Chunk 2: 295 MB - 442 MB (147 MB)
   Chunk 3: 442 MB - 590 MB (147 MB)
📥 Starting chunk 0...
📥 Starting chunk 1...
[Progress bar showing real-time download from all 4 connections]
```

### Performance Tips
1. **Always use `--release` flag** for 20-30% faster compilation-optimized code
2. **Large files benefit most** from parallel downloads (> 100 MB ideal)
3. **Resume automatically** switches to parallel if < 20% downloaded
4. **Network-bound**: Speed limited by server throttling, not client CPU

## Legal and ToS Compliance

**IMPORTANT**: This project respects website Terms of Service.

### Adding New Site Support
Before implementing support for a new website:
1. Review the site's Terms of Service
2. Ensure downloading is not explicitly prohibited
3. Check for API availability (prefer official APIs)
4. Consider rate limiting and respectful crawling

### Sites to Avoid
Do NOT add extractors for sites with:
- Explicit ToS prohibitions on downloading/automation
- DMCA-protected or copyrighted content restrictions
- Anti-scraping clauses in ToS

### Current Focus
- ✅ TNAFlix network (permissive ToS)
- ✅ Sites with official APIs
- ✅ Creative Commons / Public domain content sites
- ❌ YouTube (explicit ToS prohibition - deferred)

See **CONTRIBUTING.md** for detailed legal guidelines.
