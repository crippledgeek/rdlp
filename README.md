<div align="center">

# rdlp

**Rust Download Program** - A fast, extensible video downloader written in pure Rust

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/yourusername/rdlp)
[![Rust Version](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Crates.io](https://img.shields.io/badge/crates.io-not%20yet%20published-lightgray.svg)](https://crates.io)
[![Documentation](https://img.shields.io/badge/docs-latest-blue.svg)](docs/IMPLEMENTATION_PLAN.md)

[Features](#features) •
[Installation](#installation) •
[Quick Start](#quick-start) •
[Documentation](#documentation) •
[Contributing](#contributing)

</div>

---

## 🎯 Overview

**rdlp** is a pure Rust implementation inspired by [yt-dlp](https://github.com/yt-dlp/yt-dlp), designed from the ground up for:

- **🚀 Performance** - 37x faster downloads (10.5 MB/s) with 7-layer optimization stack
- **🔒 Memory Safety** - No segfaults or data races guaranteed by Rust
- **🧩 Extensibility** - Modular 11-crate workspace architecture with plugin support
- **📦 Small Binary** - ~8MB release binary with no runtime dependencies
- **⚡ Async Everything** - Built on tokio for efficient I/O operations
- **⚖️ ToS Compliance** - Only supports sites with permissive Terms of Service

### 🌟 Key Features at a Glance

| Category | Features |
|----------|----------|
| **Performance** | 37x faster • 10.5 MB/s downloads • Power-of-two chunking • Fine-grained parallelism |
| **Protocols** | HTTP/HTTPS • HLS with duration/fps/DRM metadata • DASH (planned) |
| **Reliability** | Auto-resume • Ctrl+C interrupt handling • Backward-compatible chunks |
| **Site Support** | 5 sites • TNAFlix • EMPFlix • MovieFap • RedTube • PornHub (with playlists) |
| **User Experience** | Interactive format selection • Segment-based progress bars • Verbose mode |
| **Post-Processing** | FFmpeg library bindings • Audio extraction • Metadata embedding • Thumbnail embedding |
| **Code Quality** | 100% Rust • 270+ tests • unsafe only in FFmpeg FFI • 0 compiler warnings |

### ⚙️ Current Status

**Production-Ready** - High-performance downloads with 37x speed improvement!

| Feature | Status | Description |
|---------|--------|-------------|
| **Power-of-Two Chunking** | ✅ Production | Memory-aligned chunks (64 KB - 8 MB) |
| **Fine-Grained Parallelism** | ✅ Production | Up to 591 concurrent chunks, 4 at a time |
| **Performance Optimizations** | ✅ Production | 37x faster (10.5 MB/s), 7-layer stack |
| HTTP Downloader | ✅ Production | Streaming downloads with progress tracking |
| Resume Support | ✅ Production | Auto-resume interrupted downloads (Ctrl+C) |
| Interactive Selection | ✅ Production | Arrow keys + ESC to choose format |
| Filesize Detection | ✅ Production | HEAD/Range requests with CDN fallback |
| **HLS Streaming** | ✅ Production | Full HLS downloads with segment-based progress |
| **FFmpeg Post-Processing** | ✅ Production | Remux, audio extraction, metadata embedding |
| **Playlist Support** | ✅ Production | Batch downloads with pagination (PornHub) |
| TNAFlix/EMPFlix/MovieFap | ✅ Production | Multi-quality extraction (144p-720p) |
| RedTube | ✅ Production | HLS + MP4 formats with segment counting |
| PornHub | ✅ Production | HLS streaming with playlist support |
| Progress Bars | ✅ Production | Segment-based for HLS, bytes for HTTP |
| Format Selection | ✅ Production | "best", "bestvideo", etc. |
| Config Files | ✅ Production | TOML/YAML support |
| More Site Support | 🚧 Planned | Vimeo, Dailymotion, Archive.org |
| Plugin System | 🚧 Planned | Dynamic extractor loading |

### 🎬 Supported Sites

Currently supporting **5 adult content sites** with **permissive Terms of Service**:

| Site | Formats | Resolutions | Size Detection | Features | Status |
|------|---------|-------------|----------------|----------|--------|
| **TNAFlix** | MP4 | 144p - 720p | HEAD + Range fallback | - | ✅ Stable |
| **EMPFlix** | MP4 | 144p - 720p | HEAD + Range fallback | - | ✅ Stable |
| **MovieFap** | MP4 | 144p - 720p | HEAD + Range fallback | - | ✅ Stable |
| **RedTube** | HLS + MP4 | Multiple variants | Fast segment counting | Segment progress | ✅ Stable |
| **PornHub** | HLS | Multiple variants | Fast segment counting | **Playlist support** | ✅ Stable |

**Extractors in Development**: None currently

**Roadmap** (Sites with permissive ToS):
- 📅 **Planned**: Vimeo, Dailymotion, Archive.org
- 🔌 **Future**: Plugin system for community-contributed extractors

> **Note**: We only add support for sites with explicit permission or permissive Terms of Service. See [Legal & ToS Compliance](#️-legal--terms-of-service) for our criteria.

## 📦 Installation

### Prerequisites

- **Rust 2024 Edition** (1.85+recommended)
- `cargo` (comes with Rust)
- Internet connection for dependencies

### Option 1: Install from Source (Current)

```bash
# Clone the repository
git clone https://github.com/yourusername/rdlp.git
cd rdlp

# Build with optimizations
cargo build --release

# Binary location: ./target/release/rdlp
# Optionally add to PATH:
export PATH="$PATH:$(pwd)/target/release"
```

### Option 2: Cargo Install (Coming Soon)

```bash
# Once published to crates.io:
cargo install rdlp
```

### Option 3: Pre-built Binaries (Coming Soon)

Download from [GitHub Releases](https://github.com/yourusername/rdlp/releases) for your platform:
- Linux (x86_64)
- macOS (Intel & Apple Silicon)
- Windows (x86_64)

### Verify Installation

```bash
# Check version
rdlp --version

# List supported extractors
rdlp --list-extractors
```

## 🚀 Quick Start

### Basic Usage

```bash
# Simple download (auto-selects best quality)
rdlp "https://www.tnaflix.com/hd-videos/video-title/video123456"

# Interactive format selection (arrow keys + ESC to cancel)
rdlp -i "https://www.redtube.com/12345678"

# Download specific quality
rdlp -f best "https://www.empflix.com/amateur-porn/title/video3715093"

# Custom output directory
rdlp -o ./downloads "https://www.moviefap.com/videos/abc123/title.html"

# Verbose mode (shows filesize detection, HEAD requests, HLS parsing)
rdlp -v "https://www.redtube.com/12345678"

# List supported sites and extractors
rdlp --list-extractors
```

**Output:**
```
Supported extractors:
  - TNAFlix: https://www.tnaflix.com/*
  - EMPFlix: https://www.empflix.com/*
  - MovieFap: https://www.moviefap.com/*
  - RedTube: https://www.redtube.com/*
  - PornHub: https://www.pornhub.com/*

Total: 5 extractors
```

### 📦 Container Remux

MPEG-TS (.ts) files from HLS streams have poor seeking. Use `--remux` to convert to a container with better seeking support:

```bash
# Interactive container selection
rdlp --remux "https://www.redtube.com/12345678"

# Direct: remux to MP4 (best compatibility, faststart)
rdlp --remux=mp4 "https://www.redtube.com/12345678"

# Direct: remux to MKV (supports all codecs)
rdlp --remux=mkv "https://www.redtube.com/12345678"

# Combine with interactive format selection
rdlp -i --remux "https://www.redtube.com/12345678"
```

**Interactive menu:**
```
? Select remux container (ESC to cancel)
> mp4    Best compatibility, faststart for streaming
  mkv    Supports all codecs, efficient cues index
  webm   Web-optimized, VP8/VP9/AV1 + Opus/Vorbis
  mov    Apple QuickTime, good for editing
  avi    Legacy format, wide support
  ts     MPEG-TS, broadcast/streaming
```

**Supported containers:**
| Container | Seeking | Use Case |
|-----------|---------|----------|
| **MP4** | Excellent (moov atom) | Best compatibility, streaming |
| **MKV** | Excellent (cues index) | All codecs, editing |
| **WebM** | Good | Web browsers, VP9/AV1 |
| **MOV** | Excellent | Apple ecosystem, editing |
| **AVI** | Fair | Legacy compatibility |
| **TS** | Poor | Broadcast, live streaming |

### 📋 Playlist Downloads

```bash
# Download entire playlist (auto-detected by URL)
rdlp "https://www.pornhub.com/playlist/123456"

# Playlist with interactive confirmation
rdlp -i "https://www.pornhub.com/playlist/123456"
```

**Output:**
```
🔍 Finding extractor for URL...
✓ Using PornHub extractor
📊 Extracting playlist information...
✓ Playlist: Best Videos Collection
✓ Found 36 videos

============================================================
📋 Playlist: Best Videos Collection
📊 Total videos: 36
============================================================

? This will download 36 videos. Continue? (y/n) › yes

────────────────────────────────────────────────────────────
📥 [1/36] Video Title One
────────────────────────────────────────────────────────────
⠏ [195/198 segments] (125.3 MB, 2.5 MB/s, 00:12)
✅ [1/36] Saved: Video Title One.mp4

[... continues for all videos ...]

============================================================
📋 Playlist Download Summary
============================================================
✅ Downloaded: 34/36
❌ Failed: 2
============================================================
```

### ⏸️ Interrupt & Resume

Press **Ctrl+C** during download to pause - progress is automatically saved!

```bash
# Start download
rdlp -i "https://www.tnaflix.com/video-url"
⚠️  Press Ctrl+C to pause and save progress
⠖ [00:00:27] [####>-------------------] 150 MiB/590 MiB

# Press Ctrl+C
^C
⏸️  Download interrupted by user
💾 Progress saved. Run the same command again to resume.

# Resume download (same command)
rdlp -i "https://www.tnaflix.com/video-url"
📋 Found partial download (150.0 MB), resuming...
⠖ [00:00:10] [#########>--------------] 300 MiB/590 MiB
```

### Example Output

```
🔍 Finding extractor for URL...
✓ Using TNAFlix extractor
📊 Extracting video information...
✓ Title: Example Video Title
✓ Found 5 formats
✓ Selected format: http-720 (720p)
💾 Downloading to: .\Example Video Title.mp4

✅ Downloaded successfully!
   File: .\Example Video Title.mp4
   Size: 96.1 MB
   Speed: 279.9 KB/s
   Time: 357.75s

🎉 Success! Video saved to: .\Example Video Title.mp4
```

### 🎨 Interactive Format Selection

Using `-i` flag shows an interactive menu with all available formats:

**Example: TNAFlix (MP4 formats)**
```
🔍 Finding extractor for URL...
✓ Using TNAFlix extractor
📊 Extracting video information...
✓ Title: Example Video Title
✓ Found 5 formats

📋 Available formats:
Quality      | Resolution | Size         | Codecs       | Protocol
-------------------------------------------------------------------------
720p         | 1280x720   | 590.2 MB     | h264/aac     | HTTP
480p         | 854x480    | 245.3 MB     | h264/aac     | HTTP
360p         | 640x360    | 128.5 MB     | h264/aac     | HTTP
240p         | 426x240    | 65.3 MB      | h264/aac     | HTTP
144p         | 256x144    | 28.1 MB      | h264/aac     | HTTP

✔ Select a format to download (ESC to cancel) ·
> 720p         | 1280x720   | 590.2 MB     | h264/aac     | HTTP
  480p         | 854x480    | 245.3 MB     | h264/aac     | HTTP
  ...
```

**Example: RedTube/PornHub (HLS formats with duration and fps)**
```
📋 Available formats:
Quality      | Resolution | Size            | Type | Codecs
-------------------------------------------------------------------------
1080p60      | 1920x1080  | 16:39 (250 seg) | HLS  | h264/aac
720p30       | 1280x720   | 16:39 (250 seg) | HLS  | h264/aac
480p30       | 854x480    | 16:39 (250 seg) | HLS  | h264/aac
240p30       | 426x240    | 16:39 (250 seg) | HLS  | h264/aac
```

**Example: DRM-protected stream**
```
Quality      | Resolution | Size            | Type | Codecs
-------------------------------------------------------------------------
1080p60      | 1920x1080  | 16:39 (250 seg) | HLS  | h264/aac [DRM]
```

**Example: Live stream warning**
```
⚠ LIVE stream detected — download may be incomplete

Quality      | Resolution | Size            | Type | Codecs
-------------------------------------------------------------------------
1080p        | 1920x1080  | LIVE            | HLS  | h264/aac
```

**Controls:**
- `↑/↓` - Navigate between formats
- `Enter` - Select and download
- `ESC` - Cancel selection (exits cleanly, no error)

> **Note**: HLS formats show segment count instead of size (fast detection). Progress bar shows segment completion during download.

## ✨ Features

### 📥 Download Features

| Feature | Description | Status |
|---------|-------------|--------|
| **Power-of-Two Chunking** | Memory-aligned chunks (64 KB - 8 MB) targeting ~1024 chunks | ✅ |
| **Fine-Grained Parallelism** | Up to 591 concurrent chunks processed 4 at a time | ✅ |
| **37x Faster Downloads** | 590 MB in 56.4s (10.5 MB/s) with 7-layer optimization stack | ✅ |
| **HLS Streaming** | Full HLS with duration-based progress and rich metadata | ✅ |
| **HLS Metadata** | Duration, fps, DRM badge, live detection, container detection | ✅ |
| **Playlist Support** | Batch downloads with pagination and progress tracking | ✅ |
| **Multi-Quality** | Automatic detection of 144p to 1080p formats | ✅ |
| **Smart Filesize** | HEAD/Range requests with CDN fallback | ✅ |
| **Streaming** | Constant memory usage (~13MB) regardless of video size | ✅ |
| **Resume Support** | Ctrl+C to pause, auto-resume on restart (backward-compatible) | ✅ |
| **Progress Bars** | Segment-based for HLS, bytes-based for HTTP | ✅ |
| **Format Selection** | `-f best`, `-f bestvideo`, or interactive menu | ✅ |
| **FFmpeg Post-Processing** | Remux HLS to MP4, extract audio, embed metadata | ✅ |

### 🖥️ CLI Features

| Feature | Description | Example |
|---------|-------------|---------|
| **Interactive Menu** | Arrow keys to select, ESC to cancel | `-i` |
| **Remux Container** | Convert to MP4/MKV for better seeking | `--remux` or `--remux=mp4` |
| **Verbose Mode** | Shows HEAD requests, debug info | `-v` |
| **Quiet Mode** | Minimal output for scripts | `-q` |
| **Simulate Mode** | Test extraction without downloading | `-s` |
| **Output Directory** | Custom download location | `-o ./videos` |
| **Config Files** | TOML/YAML configuration | `config.toml` |
| **Clean Output** | Color-coded status with emojis | Default |

### 🏗️ Code Quality

| Metric | Value | Notes |
|--------|-------|-------|
| **Build Status** | ✅ Passing | Zero compiler warnings (release builds) |
| **Test Coverage** | 270+ tests | All passing (unit + integration + property tests) |
| **Architecture** | 11 crates | Clean separation of concerns |
| **Type Safety** | 100% | `unsafe` only in FFmpeg FFI (18 sites, all documented) |
| **Documentation** | Full | Inline docs + comprehensive guides |
| **Rust Files** | 50+ files | Pure Rust (2024 Edition) |

## Architecture

### Workspace Structure

rdlp uses a modular workspace with 11 specialized crates:

```
rdlp/
├── crates/
│   ├── rdlp-types/        # Pure data types (no I/O) - Foundation
│   ├── rdlp-core/         # Traits, errors - Contracts
│   ├── rdlp-security/     # SSRF protection, URL validation
│   ├── rdlp-http/         # Centralized HTTP client factory
│   ├── rdlp-extractor/    # Site-specific extractors
│   ├── rdlp-downloader/   # Protocol handlers (HTTP, HLS)
│   ├── rdlp-jsinterp/     # JavaScript execution (boa)
│   ├── rdlp-postprocess/  # FFmpeg library bindings pipeline
│   ├── rdlp-cookies/      # Browser cookie extraction
│   ├── rdlp-plugin/       # Dynamic plugin loading
│   └── rdlp-cli/          # User-facing CLI
└── docs/                  # Documentation
```

### Three-Stage Pipeline

```
URL → Extraction → Download → Post-Processing → Video File
      ↓            ↓           ↓
      Metadata     Streaming   FFmpeg library bindings
```

**Extraction Stage:**
- Parse URL and find appropriate extractor
- Fetch video page HTML
- Extract metadata (title, description, uploader, thumbnail)
- Parse video sources and quality levels
- Build `InfoDict` with format list

**Download Stage:**
- Select format based on user preferences
- Choose appropriate protocol handler
- Stream video data with progress tracking
- Support resume via HTTP Range requests
- Calculate download statistics

**Post-Processing Stage:**
- Remux HLS segments to MP4 container (automatic for HLS downloads)
- Container remux for better seeking (TS → MP4/MKV/WebM/MOV/AVI)
- Extract audio to MP3/AAC/Opus/FLAC/WAV
- Embed metadata (title, artist, date, chapters)
- Embed thumbnail as cover art (MP4/MKV/MP3/FLAC)
- Priority-based processor pipeline (100→50→45→40→30→20)

### Key Design Decisions

#### Why Rust?
- **Memory Safety**: No segfaults or data races
- **Performance**: Zero-cost abstractions, native speed
- **Concurrency**: Fearless concurrency with async/await
- **Ecosystem**: Rich ecosystem (tokio, reqwest, scraper)

#### Why Workspace Architecture?
- **Modularity**: Clear separation of concerns
- **Testability**: Independent component testing
- **Reusability**: Libraries can be used independently
- **Parallel Builds**: Faster compilation with cargo

#### Why HTML Parsing over XML?
- **Simplicity**: Direct extraction from video player tags
- **Reliability**: No dependency on deprecated XML endpoints
- **Performance**: Single HTTP request instead of two
- **Maintainability**: Easier to adapt to site changes

## Development

### Building

```bash
# Development build (faster compilation, no optimizations)
cargo build

# Release build (optimized for performance)
cargo build --release

# Check code without building
cargo check

# Run all tests
cargo test

# Run tests for specific crate
cargo test -p rdlp-core

# Run clippy linter
cargo clippy

# Format code
cargo fmt
```

### Testing

```bash
# Run all tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_tnaflix_url_pattern

# Run tests in single-threaded mode
cargo test -- --test-threads=1

# Run tests quietly
cargo test --quiet
```

### Code Quality Checks

```bash
# Check for warnings
cargo clippy -- -W clippy::all

# Check formatting without changes
cargo fmt -- --check

# Build documentation
cargo doc --no-deps --open
```

## 📋 Implementation Status

### Phase 1: Foundation ✅ COMPLETE
- [x] Core traits (InfoExtractor, Downloader, PostProcessor)
- [x] InfoDict and Format structures
- [x] Error handling with thiserror
- [x] Config with TOML/YAML support
- [x] All workspace crates initialized

### Phase 2: TNAFlix Support ✅ COMPLETE
- [x] HTTP downloader with streaming
- [x] Progress tracking with indicatif
- [x] Resume capability (Range requests, auto-resume)
- [x] TNAFlix/EMPFlix/MovieFap extractors
- [x] HTML parsing for video sources
- [x] Format selection (basic + interactive)
- [x] CLI orchestrator
- [x] Ctrl+C interrupt handling with progress save
- [x] Filesize detection via HEAD/Range requests
- [x] **Status**: Production-ready, fully tested

### Phase 2.5: Power-of-Two Chunking ✅ COMPLETE (2026-01-19)
- [x] Core chunk sizing algorithm (targets ~1024 chunks)
- [x] ChunkSizeStrategy enum (Auto, Fixed, Legacy)
- [x] 13 unit tests + 4 property-based tests
- [x] Fine-grained parallelism with buffer_unordered
- [x] Unique download IDs to prevent collisions
- [x] Graceful cleanup on success and error
- [x] **Performance**: 37x faster (590 MB in 56.4s at 10.5 MB/s)
- [x] **Status**: Production-ready, validated with real-world downloads

### Phase 3: Resume Compatibility ✅ COMPLETE (2026-01-19)
- [x] Backward-compatible chunk detection
- [x] ChunkInfo struct tracking old/new formats
- [x] Intelligent chunk merging (up to 10,000 chunks)
- [x] Automatic cleanup of obsolete chunks
- [x] Smart prioritization (new over old)
- [x] 6 backward compatibility tests
- [x] **Status**: 100% compatible with old chunk format

### Phase 4: HLS Size Detection ✅ COMPLETE (2026-01-20)
- [x] HlsSizeDetector with configurable concurrency
- [x] M3U8 playlist parsing (master + media)
- [x] Parallel segment size fetching (8 concurrent)
- [x] HEAD request with Range fallback
- [x] Security limits (max 10,000 segments)
- [x] RedTube extractor integration
- [x] 3 unit tests + real-world validation
- [x] **Performance**: 2-5s per HLS format (300-800 segments)
- [x] **Status**: Production-ready

### Phase 5: PornHub Playlist Support ✅ COMPLETE (2026-01-22)
- [x] PornHub video extractor with HLS support
- [x] Playlist extraction with pagination (AJAX endpoint)
- [x] Page calculation formula from yt-dlp
- [x] Rate limiting (500ms between pages)
- [x] Video deduplication with HashSet
- [x] Graceful degradation (skip failed videos)
- [x] **Status**: Production-ready, full playlist support

### Phase 6: CLI Playlist Integration ✅ COMPLETE (2026-01-22)
- [x] Auto-detection of playlists vs single videos
- [x] Batch download workflow with confirmation
- [x] Per-video progress tracking [N/total]
- [x] Summary report with success/failure counts
- [x] Ctrl+C interrupt handling
- [x] **Status**: Production-ready

### Phase 6.5: HLS Progress & Segment Counting ✅ COMPLETE (2026-01-24)
- [x] Segment-based progress tracking (195/198 segments)
- [x] Fast segment counting (~100-500ms, no size fetching)
- [x] Format table shows segment count for HLS formats
- [x] Dynamic progress bar adjustment
- [x] Leftover segment cleanup before downloads
- [x] All extractors updated (PornHub, RedTube)
- [x] **Status**: Production-ready, 213 tests passing

### Phase 7: Post-Processing Pipeline ✅ COMPLETE (2026-01-23)
- [x] FFmpegRunner with auto-detection
- [x] FFmpegMerger - Remux HLS to MP4
- [x] FFmpegExtractAudio - Audio extraction with quality settings
- [x] FFmpegVideoConvertor - Remux or transcode
- [x] FFmpegMetadata - Embed title, artist, chapters
- [x] EmbedThumbnail - Cover art for MP4/MKV/MP3
- [x] Priority-based processor registry
- [x] **Status**: Production-ready, 26 tests passing

### Phase 8: FFmpeg Library Migration ✅ COMPLETE (2026-01-28)
- [x] Migrated all FFmpeg operations from CLI process spawning to `ffmpeg-the-third` v4.0 library bindings
- [x] Replaced `ffprobe` CLI with library-based `probe_sync()`
- [x] Replaced all 5 processor CLI invocations with library calls (remux, audio extract, video convert, metadata, thumbnail)
- [x] Removed `which`, `regex`, `serde_json` CLI dependencies
- [x] Fixed 5 bugs during migration (config ignored, unconditional AAC BSF, MP3 thumbnail, description truncation, lost InfoDict)
- [x] Post-migration quality fixes: unwrap panic elimination (28 sites), async I/O correctness, download timeouts, resilience improvements
- [x] **Status**: Production-ready, 270+ tests passing, 0 clippy warnings

### Phase 9: HLS Metadata Surface ✅ COMPLETE (2026-01-28)
- [x] Duration field with mm:ss display alongside segment count
- [x] FPS in quality column (e.g., 1080p60) for non-standard frame rates
- [x] DRM badge [DRM] for encrypted streams
- [x] DRM formats filtered from automatic "best" selection
- [x] LIVE stream warning before format selection
- [x] is_live flag propagation from M3U8 to InfoDict
- [x] Duration-based progress tracking (more accurate than segment count)
- [x] FPS as fourth-tier ranking tiebreaker (quality > height > bitrate > fps)
- [x] Container detection from segment URLs (ts/mp4/m4s)
- [x] **Status**: Production-ready, 270+ tests passing

### Phases 10-12: Future
- Additional site extractors (Vimeo, Dailymotion, Archive.org)
- Enhanced CLI features
- Format selection DSL parser
- Browser cookie extraction
- Plugin system
- Polish and release

See [CLAUDE.md](CLAUDE.md) for detailed architecture and implementation notes.

## Configuration

### Config File (Optional)

Create a `config.toml` file:

```toml
# Output options
output_template = "%(title)s.%(ext)s"
output_directory = "./downloads"
restrict_filenames = false
overwrite = false

# Download options
format = "best"
concurrent_fragments = 5
buffer_size = 2097152  # 2 MB

# Display options
quiet = false
verbose = false
progress = true
```

### Environment Variables

```bash
# Set default output directory
export RDLP_OUTPUT_DIR="./videos"

# Enable verbose mode by default
export RDLP_VERBOSE=1
```

## Troubleshooting

### Video Not Found

```
❌ Error: Failed to extract video information
Caused by: Network error: HTTP error 404: Not Found. Video may be unavailable.
```

**Solution**: The video may be deleted or restricted. Try another video.

### Network Timeout

```
❌ Error: Network error: Failed to fetch webpage: ...
```

**Solution**: Check your internet connection or try again later.

### Invalid URL

```
❌ Error: No extractor found for URL: https://example.com/video
```

**Solution**: Make sure the URL is from a supported site (TNAFlix, EMPFlix, or MovieFap).

### Permission Denied

```
❌ Error: I/O error: Permission denied
```

**Solution**: Check write permissions for the output directory.

## ⚡ Performance

### Benchmarks

Tested on Windows 11, Intel Core i7, 1 Gbps connection:

| Optimization Level | File Size | Time | Speed | Improvement |
|-------------------|-----------|------|-------|-------------|
| **Baseline** (8KB buffer, single thread) | 590 MB | 35+ min | ~360 KB/s | 1x |
| **+ Buffered I/O** (2MB buffer) | 590 MB | ~15 min | ~650 KB/s | 2x |
| **+ Connection pooling** | 590 MB | ~12 min | ~820 KB/s | 2.5x |
| **+ Parallel chunks** (4 connections) | 590 MB | ~9 min | ~1.1 MB/s | 3x |
| **+ Multi-threaded runtime** | 590 MB | ~6-8 min | ~1.5 MB/s | 4-5x |
| **+ Power-of-two chunking** 🚀 | 590 MB | **56.4s** | **10.5 MB/s** | **37x** |

**Latest Test Results (Phase 2.5):**
- **File size**: 590 MB
- **Chunk size**: 1 MB (power-of-two aligned)
- **Total chunks**: 591 chunks
- **Concurrent connections**: 4 (batched processing)
- **Download time**: 56.4 seconds
- **Average speed**: 10.5 MB/s
- **Speedup**: 37x faster than baseline

### Additional Performance Metrics

| Operation | Time | Notes |
|-----------|------|-------|
| URL Pattern Matching | < 1ms | Compiled regex |
| Extraction (HTML parse) | 50-100ms | Network dependent |
| **Fast Segment Counting** | 100-500ms | Parses m3u8 only (no size fetching) |
| HLS Full Size Detection | 2-5s | 300-800 segments, 8 concurrent HEAD requests |
| Format Selection | < 1ms | In-memory comparison |
| Chunk Cleanup | ~100ms | 591 files removed automatically |
| FFmpeg Remux (HLS→MP4) | 1-5s | Copy streams, no re-encoding |

### Memory Usage

- **Base**: ~5 MB (CLI runtime)
- **Extraction**: ~10 MB (HTML parsing)
- **Download**: ~13 MB (2MB streaming buffer)
- **Peak**: < 20 MB total

All downloads use streaming to maintain constant memory usage regardless of video size.

### 7-Layer Optimization Stack

1. **Power-of-Two Chunk Sizing** - Memory-aligned chunks (64 KB - 8 MB)
2. **Fine-Grained Parallel Downloads** - Up to 591 chunks, 4 concurrent
3. **Multi-Threaded Tokio Runtime** - 2x CPU cores (16 threads on 8-core CPU)
4. **Buffered I/O** - 2MB buffers (250x larger than baseline)
5. **HTTP Client Optimizations** - Connection pooling, TCP_NODELAY, smart timeouts
6. **Intelligent Size Detection** - HEAD requests with Range fallback
7. **Real-Time Progress Tracking** - 100ms updates with atomic counters

## 🤝 Contributing

**We welcome contributions!** Whether you're fixing bugs, adding features, improving docs, or adding site extractors.

### 🎯 Good First Issues

Perfect for newcomers to the Rust ecosystem:

- [x] ~~Add retry logic with exponential backoff~~ (done — `backon` crate)
- [ ] Add rate limiting to HTTP downloader (`rdlp-downloader`)
- [ ] Improve error messages with suggestions (`rdlp-core`)
- [ ] Add more unit tests for edge cases (all crates)
- [ ] Write integration tests for extractors (`rdlp-extractor`)
- [ ] Improve CLI help text and examples (`rdlp-cli`)
- [ ] Add shell completions: bash, zsh, fish (`rdlp-cli`)
- [ ] Add progress callback tests (`rdlp-downloader`)
- [ ] Document extractor API with examples (`rdlp-extractor`)
- [ ] Add benchmarks for chunking algorithm (`rdlp-downloader`)

### 🏗️ Areas That Need Help

| Area | Difficulty | Crate(s) | Description | Examples |
|------|------------|----------|-------------|----------|
| **Extractors** | 🟢 Easy | `rdlp-extractor` | Add support for new sites | See `tnaflix.rs`, `pornhub.rs` as templates |
| **Testing** | 🟢 Easy | All crates | Write unit/integration tests | Add tests for edge cases, error handling |
| **Documentation** | 🟢 Easy | All crates | Examples, guides, API docs | Inline docs, CLAUDE.md updates, tutorials |
| **CLI UX** | 🟡 Medium | `rdlp-cli` | Better error messages, progress display | Colored output, better help text |
| **Performance** | 🟡 Medium | `rdlp-downloader` | Benchmarking, profiling, optimizations | Criterion benchmarks, flamegraphs |
| **AES-128 Decryption** | 🟡 Medium | `rdlp-downloader` | Encrypted HLS stream support | Key fetching, segment decryption |
| **DASH Protocol** | 🔴 Hard | `rdlp-downloader` | Adaptive streaming (DASH MPD) | MPD parsing, segment selection |
| **Plugin System** | 🔴 Hard | `rdlp-plugin` | Dynamic extractor loading | libloading, safe FFI, versioning |

### 🔧 How to Add a New Extractor

1. **Create extractor file**: `crates/rdlp-extractor/src/extractors/yoursite.rs`
2. **Implement `InfoExtractor` trait**:
   ```rust
   pub struct YourSiteExtractor;

   #[async_trait]
   impl InfoExtractor for YourSiteExtractor {
       fn name(&self) -> &str { "YourSite" }

       fn url_patterns(&self) -> Vec<Regex> {
           vec![Regex::new(r"https?://(?:www\.)?yoursite\.com/.*").unwrap()]
       }

       async fn extract(&self, url: &str, ctx: &Context) -> Result<InfoDict> {
           // 1. Fetch webpage HTML
           // 2. Parse with scraper
           // 3. Extract video URLs, metadata
           // 4. Build Format list
           // 5. Return InfoDict
       }
   }
   ```
3. **Register in `mod.rs`**: Add to extractor list
4. **Write tests**: Add URL pattern tests, mock extraction tests
5. **Update docs**: Add to supported sites list

See [`CLAUDE.md`](CLAUDE.md) for detailed architecture and [`tnaflix.rs`](crates/rdlp-extractor/src/extractors/tnaflix.rs) for a complete example.

### 📋 Development Workflow

1. **Fork & Clone**
   ```bash
   git clone https://github.com/yourusername/rdlp.git
   cd rdlp
   ```

2. **Create Branch**
   ```bash
   git checkout -b feature/my-awesome-feature
   ```

3. **Make Changes**
   - Check [IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) for architecture
   - Follow existing code style
   - Write tests first (TDD)

4. **Test & Lint**
   ```bash
   cargo test           # Run all tests
   cargo clippy         # Check for issues
   cargo fmt            # Format code
   cargo build --release # Verify build
   ```

5. **Commit & Push**
   ```bash
   git add .
   git commit -m "feat: add awesome feature"
   git push origin feature/my-awesome-feature
   ```

6. **Open Pull Request**
   - Describe what changed and why
   - Reference any related issues
   - Add screenshots/examples if applicable

### 📝 Commit Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `test:` - Adding tests
- `refactor:` - Code restructuring
- `perf:` - Performance improvements
- `chore:` - Maintenance tasks

### 🔍 Code Review Process

1. Automated checks (build, tests, clippy) must pass
2. At least one maintainer review required
3. Address feedback, then merge

### 📚 Resources

- [Rust Book](https://doc.rust-lang.org/book/) - Learn Rust
- [tokio Tutorial](https://tokio.rs/tokio/tutorial) - Async Rust
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) - Reference implementation
- [CLAUDE.md](CLAUDE.md) - Project conventions

## ❓ FAQ

### How does rdlp compare to yt-dlp?

| Feature | rdlp | yt-dlp |
|---------|------|--------|
| **Language** | Pure Rust (2024) | Python 3.8+ |
| **Binary Size** | ~8 MB | ~100 MB (with dependencies) |
| **Memory Usage** | ~13 MB (constant streaming) | Varies (can be high) |
| **Download Speed** | 10.5 MB/s (37x optimized) | Varies by implementation |
| **Startup Time** | Instant | ~1-2s (Python import) |
| **Supported Sites** | 5 (growing) | 1800+ |
| **Resume Support** | ✅ Native (Range requests) | ✅ Native |
| **Parallel Downloads** | ✅ Power-of-two chunking | ✅ Fragment-based |
| **HLS Support** | ✅ Full (with segment progress) | ✅ Full support |
| **Playlist Support** | ✅ PornHub (pagination) | ✅ Many sites |
| **DASH Support** | ❌ Planned | ✅ Full support |
| **FFmpeg Integration** | ✅ Remux, audio, metadata | ✅ Full support |
| **Plugin System** | 🚧 Planned | ✅ Native (Python imports) |
| **ToS Compliance** | ✅ Focus on permissive sites | ⚠️ Supports all sites |
| **Type Safety** | ✅ 100% (Rust compiler) | ⚠️ Optional (type hints) |
| **Maturity** | Alpha (stable for 5 sites) | Production (battle-tested) |

**When to choose rdlp:**
- ✅ You need maximum performance (37x faster downloads)
- ✅ You want minimal memory footprint (~13 MB constant)
- ✅ You prefer type-safe, compiled binaries
- ✅ You care about ToS compliance
- ✅ You're downloading from supported sites (TNAFlix, RedTube, PornHub, etc.)
- ✅ You want segment-based progress for HLS streams

**When to choose yt-dlp:**
- ✅ You need broad site support (1800+ sites)
- ✅ You need mature, battle-tested software
- ✅ You need DASH support right now
- ✅ You prefer Python extensibility
- ✅ You need features like subtitle extraction, browser cookies, etc.

**Bottom Line**: rdlp is faster and more memory-efficient with stronger type safety. It now has full HLS support with FFmpeg post-processing. yt-dlp has far more site support and maturity. Choose based on your needs!

### Why Rust?

- **Memory Safety**: No segfaults, data races, or buffer overflows
- **Performance**: Native speed without garbage collection
- **Concurrency**: Fearless async/await with tokio
- **Reliability**: If it compiles, it usually works
- **Modern**: Great tooling (cargo, clippy, rustfmt)

### Can I use rdlp in production?

**Yes, for supported sites!** The current version is production-ready for TNAFlix, EMPFlix, MovieFap, RedTube, and PornHub. Features like resume, progress tracking, format selection, HLS streaming, and FFmpeg post-processing are stable and well-tested.

**Note on site support**: rdlp prioritizes sites with permissive Terms of Service. We're expanding to Vimeo, Dailymotion, Archive.org, and other platforms that explicitly allow or don't prohibit downloading.

### How can I add support for a new site?

**Step-by-Step Guide:**

1. **Check ToS Compliance** - Ensure the site allows downloading (see [CONTRIBUTING.md](CONTRIBUTING.md))
2. **Study existing extractors**:
   - [`tnaflix.rs`](crates/rdlp-extractor/src/extractors/tnaflix.rs) - MP4 extraction with XML config
   - [`redtube.rs`](crates/rdlp-extractor/src/extractors/redtube.rs) - HLS + MP4 with size detection
3. **Implement `InfoExtractor` trait**:
   ```rust
   pub struct YourSiteExtractor;

   #[async_trait]
   impl InfoExtractor for YourSiteExtractor {
       fn name(&self) -> &str { "YourSite" }

       fn url_patterns(&self) -> Vec<Regex> {
           vec![Regex::new(r"^https?://(?:www\.)?yoursite\.com/").unwrap()]
       }

       async fn extract(&self, url: &str, ctx: &Context) -> Result<InfoDict> {
           // Implementation here
       }
   }
   ```
4. **Parse HTML with `scraper`**:
   ```rust
   let html = Html::parse_document(&webpage);
   let selector = Selector::parse("video source").unwrap();
   ```
5. **Register in `crates/rdlp-extractor/src/extractors/mod.rs`**
6. **Write tests** - URL patterns, mock extraction
7. **Update documentation** - Add to supported sites table
8. **Submit PR** with description of extraction method

**Resources:**
- [CLAUDE.md](CLAUDE.md) - Architecture and patterns
- [CONTRIBUTING.md](CONTRIBUTING.md) - Legal guidelines
- [InfoExtractor trait docs](crates/rdlp-core/src/traits/extractor.rs)

### Does rdlp support HLS/DASH?

**Full HLS support**:
- ✅ **HLS Streaming** - Complete download with duration-based progress
- ✅ **Rich Metadata** - Duration (mm:ss), fps, DRM badge, live stream detection
- ✅ **Smart Format Selection** - Ranking: quality > height > bitrate > fps
- ✅ **Container Detection** - Automatic detection from segment URLs (ts/mp4/m4s)
- ✅ **FFmpeg Remux** - Automatic HLS→MP4 conversion
- ❌ **DASH** - Not yet supported (planned for future phases)

**Current capabilities:**
- RedTube and PornHub extractors with full HLS download support
- Duration-based progress tracking with accurate ETA
- Format table shows duration, fps, and DRM status
- LIVE stream warning before download
- DRM formats filtered from automatic "best" selection
- Automatic container detection from segment metadata

**Coming soon:** DASH/MPD support, AES-128 decryption

### Can I cancel a download without losing progress?

**Yes!** Press **Ctrl+C** during download. Progress is saved automatically, and re-running the same command will resume from where you left off.

### Why does filesize show 0 MB sometimes?

Some CDNs don't respond to HEAD requests properly. rdlp automatically falls back to Range requests to detect file size. Run with `-v` flag to see debug info.

## ⚖️ Legal & Terms of Service

**rdlp respects website Terms of Service and prioritizes ethical site support.**

### Site Selection Criteria

We **only** add extractors for sites that meet at least one of these criteria:

| Criterion | Description | Examples |
|-----------|-------------|----------|
| **Public Domain** | Content explicitly in the public domain | Archive.org, Wikimedia Commons |
| **Permissive ToS** | Terms explicitly allow or don't prohibit downloading | Vimeo (some videos), Dailymotion |
| **Creator Intent** | Site designed for content distribution/hosting | Self-hosted platforms, creator sites |
| **Educational/Archive** | Non-commercial archival or educational use allowed | Educational platforms, research archives |
| **Adult Content** | Sites with permissive ToS for adult content | TNAFlix, EMPFlix, MovieFap, RedTube |

### Sites We Avoid

❌ **We do NOT support**:
- Sites with explicit ToS prohibitions against downloading or scraping
- Major streaming services with DRM (Netflix, Disney+, HBO Max, etc.)
- Sites with anti-automation clauses in their ToS
- Platforms with DMCA-protected or copyright-restricted content
- Any site where downloading would violate their Terms of Service

### User Responsibility

⚠️ **Important**: Users are responsible for:
- Complying with applicable laws in their jurisdiction
- Respecting website Terms of Service
- Only downloading content they have permission to download
- Respecting copyright and intellectual property rights

**rdlp is a tool**. Like `curl` or `wget`, it can be used responsibly or irresponsibly. We encourage responsible use and will not add support for sites that explicitly prohibit downloading.

### Contributing New Extractors

Before submitting a PR for a new site extractor:

1. ✅ **Read the site's Terms of Service** thoroughly
2. ✅ **Verify downloading is allowed** or not prohibited
3. ✅ **Check for official APIs** (prefer APIs over scraping)
4. ✅ **Document your research** in the PR description
5. ✅ **Add site to supported sites table** with ToS compliance notes

See [CONTRIBUTING.md](CONTRIBUTING.md) for detailed legal guidelines.

### ToS Compliance Examples

| Site | Status | Rationale |
|------|--------|-----------|
| **Archive.org** | ✅ Planned | Explicitly allows downloading for archival purposes |
| **Vimeo** | ✅ Planned | Offers download option for some videos, permissive ToS |
| **Dailymotion** | ✅ Planned | Allows downloading where creators enable it |
| **TNAFlix Network** | ✅ Supported | Permissive ToS, adult content hosting |
| **RedTube** | ✅ Supported | Permissive ToS, adult content hosting |
| **PornHub** | ✅ Supported | Permissive ToS, adult content hosting (with playlists) |
| **YouTube** | ❌ Deferred | Explicit ToS prohibition against downloading (Section 4.B) |
| **Netflix/Disney+** | ❌ Never | DRM-protected, explicit ToS violations |

## 🔒 Security

### Reporting Vulnerabilities

**Do NOT open public issues for security vulnerabilities.**

Email security reports to: `security@yourproject.com` (replace with actual email)

We take security seriously and will respond within 48 hours.

### Security Features

- ✅ Minimal unsafe (only FFmpeg FFI in `ffmpeg.rs`, all with SAFETY comments)
- ✅ Input validation on all URLs
- ✅ Sanitized filenames (no path traversal)
- ✅ HTTPS by default (rustls)
- ✅ No shell command execution (FFmpeg uses library bindings, not CLI)
- ✅ Dependency auditing with `cargo audit`

## 📜 License

Dual-licensed under your choice of:

- **MIT License** ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
- **Apache License 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)

This means you can use rdlp in commercial or open source projects without restrictions.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

## 🙏 Acknowledgments

This project wouldn't be possible without these amazing open source projects:

### Inspiration
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)** - Feature-rich downloader that inspired this project
- **[youtube-dl](https://github.com/ytdl-org/youtube-dl)** - The original that started it all

### Core Dependencies
- **[tokio](https://tokio.rs/)** - Blazing fast async runtime
- **[reqwest](https://github.com/seanmonstar/reqwest)** - Elegant HTTP client
- **[scraper](https://github.com/causal-agent/scraper)** - Fast HTML parsing
- **[indicatif](https://github.com/console-rs/indicatif)** - Beautiful progress bars
- **[dialoguer](https://github.com/console-rs/dialoguer)** - Interactive CLI prompts
- **[clap](https://github.com/clap-rs/clap)** - Command-line argument parsing

### Community
- **Rust Community** - For amazing tools and documentation
- **Contributors** - Everyone who has submitted PRs, issues, and feedback

## 📊 Project Stats

| Metric | Value |
|--------|-------|
| **Language** | 100% Rust (2024 Edition) |
| **Rust Files** | 50+ source files |
| **Crates** | 11 (modular workspace) |
| **Dependencies** | 30+ (workspace-level, including ffmpeg-the-third, m3u8-rs, futures) |
| **Tests** | 270+ tests (all passing) |
| **Build Time** | ~10s (clean release), ~2s (incremental) |
| **Binary Size** | ~8 MB (release, no runtime dependencies) |
| **Unsafe Code** | FFmpeg FFI only (18 sites in `ffmpeg.rs`, all SAFETY-documented) |
| **Download Speed** | 10.5 MB/s (37x faster than baseline) |
| **Status** | Alpha (Production-ready for 5 sites) |

## 🗺️ Roadmap

### ✅ Completed (2026-01-28)
- **Phase 1: Foundation** - Core traits, error handling, 11-crate workspace architecture
- **Phase 2: TNAFlix Support** - HTTP downloader, streaming, resume, progress tracking
- **Phase 2.5: Power-of-Two Chunking** - 37x faster downloads with memory-aligned chunks
- **Phase 3: Resume Compatibility** - Backward-compatible with old chunk format
- **Phase 4: HLS Size Detection** - M3U8 playlist parsing with parallel segment fetching
- **Phase 5: PornHub Playlist Support** - Full playlist extraction with pagination
- **Phase 6: CLI Playlist Integration** - Batch downloads with [N/total] progress
- **Phase 6.5: HLS Progress & Segment Counting** - Fast segment counting, segment-based progress
- **Phase 7: Post-Processing Pipeline** - FFmpeg library bindings (remux, audio, metadata, thumbnails)
- **Phase 8: FFmpeg Library Migration** - Replaced all CLI FFmpeg with `ffmpeg-the-third` library calls
- **Phase 9: HLS Metadata Surface** - Duration, fps, DRM badge, live warning, container detection
- **Quality Fixes** - Unwrap elimination, async I/O correctness, download timeouts, resilience improvements
- **TNAFlix Network** - Support for TNAFlix, EMPFlix, MovieFap (MP4 downloads)
- **RedTube Support** - HLS + MP4 formats with segment counting
- **PornHub Support** - HLS streaming with full playlist support
- **Interactive CLI** - Format selection with arrow keys + ESC cancellation
- **Smart Filesize** - HEAD/Range request detection with CDN fallback
- **Performance Optimizations** - 7-layer optimization stack (10.5 MB/s downloads)

### 📅 Planned
- **More Extractors** - Vimeo, Dailymotion, Archive.org, and more
- **Enhanced CLI** - Better format selection DSL, improved UX
- **Browser Cookies** - Extract cookies for authenticated downloads
- **Plugin System** - Dynamic loading of custom extractors
- **Subtitle Support** - Download and embed subtitles/captions
- **AES-128 Decryption** - Encrypted HLS stream support
- **v1.0.0 Release** - Stable API, comprehensive docs, binary releases

See [IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) for detailed technical roadmap.

## 🌟 Star History

If you find rdlp useful, please consider starring the repository! It helps others discover the project.

[![Star History Chart](https://api.star-history.com/svg?repos=yourusername/rdlp&type=Date)](https://star-history.com/#yourusername/rdlp&Date)

---

<div align="center">

**[⬆ Back to Top](#rdlp)**

Made with ❤️ and 🦀 by the rdlp contributors

**Status**: Alpha (Production-Ready for 5 Sites) | **License**: MIT OR Apache-2.0 | **Rust**: 2024 Edition | **Performance**: 37x Faster

[Report Bug](https://github.com/yourusername/rdlp/issues) · [Request Feature](https://github.com/yourusername/rdlp/issues) · [Discussions](https://github.com/yourusername/rdlp/discussions)

</div>
