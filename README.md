# rdlp

**Rust Download Program** - A fast, extensible video downloader written in Rust.

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/crippledgeek/rdlp)
[![Rust Version](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Inspired by [yt-dlp](https://github.com/yt-dlp/yt-dlp). Built on tokio, reqwest, and FFmpeg library bindings.

## Features

- **Performance** - 10+ MB/s downloads with parallel chunking and power-of-two sizing
- **HLS support** - Full streaming with duration-based progress, DRM detection, live stream warnings
- **Post-processing** - FFmpeg library bindings for remux, audio extraction, metadata/thumbnail embedding
- **Resume** - Ctrl+C to pause, re-run to resume automatically
- **Interactive** - Arrow-key format selection, container remux menu
- **Playlists** - Batch downloads with pagination (PornHub)
- **Cookie support** - Browser extraction (Chrome, Firefox) and Netscape cookie files

### Supported Sites

| Site | Formats | Features |
|------|---------|----------|
| TNAFlix | MP4 | Multi-quality (144p-720p) |
| EMPFlix | MP4 | Multi-quality (144p-720p) |
| MovieFap | MP4 | Multi-quality (144p-720p) |
| RedTube | HLS + MP4 | Segment-based progress |
| PornHub | HLS | Playlist support |

## Installation

Requires **Rust 2024 Edition** (1.85+).

```bash
git clone https://github.com/crippledgeek/rdlp.git
cd rdlp
cargo build --release
# Binary: ./target/release/rdlp
```

## Usage

```bash
# Basic download (auto-selects best quality)
rdlp "https://www.redtube.com/12345678"

# Interactive format selection
rdlp -i "https://www.redtube.com/12345678"

# Remux HLS to MP4/MKV for better seeking
rdlp --remux "https://www.redtube.com/12345678"
rdlp --remux=mp4 "https://www.redtube.com/12345678"

# Playlist download
rdlp "https://www.pornhub.com/playlist/123456"

# With browser cookies
rdlp --cookies-from-browser chrome "https://www.pornhub.com/view_video.php?viewkey=..."

# With cookie file (Netscape format)
rdlp --cookies cookies.txt "https://www.pornhub.com/view_video.php?viewkey=..."

# Verbose mode
rdlp -v "https://www.redtube.com/12345678"
```

### Resume

Press **Ctrl+C** during download to pause. Re-run the same command to resume from where you left off.

## Architecture

11-crate workspace with a three-stage pipeline: **Extract** -> **Download** -> **Post-process**.

```
rdlp/
├── crates/
│   ├── rdlp-types/        # Pure data types (Config, Format, InfoDict)
│   ├── rdlp-core/         # Traits (InfoExtractor, Downloader, PostProcessor)
│   ├── rdlp-security/     # SSRF protection, URL validation
│   ├── rdlp-http/         # HTTP client factory
│   ├── rdlp-extractor/    # Site extractors
│   ├── rdlp-downloader/   # HTTP + HLS downloaders
│   ├── rdlp-postprocess/  # FFmpeg library bindings pipeline
│   ├── rdlp-cookies/      # Browser cookie extraction (Chrome, Firefox, Netscape)
│   ├── rdlp-jsinterp/     # JavaScript interpreter (boa)
│   ├── rdlp-plugin/       # Plugin system
│   └── rdlp-cli/          # CLI application and orchestrator
└── docs/                  # Documentation
```

See [docs/](docs/README.md) for architecture analysis, protocol docs, and implementation details.

## Development

```bash
cargo build --release      # Optimized build
cargo test                 # Run all tests (270+)
cargo clippy               # Lint check
cargo fmt                  # Format code
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for adding extractors, code style, and development workflow.
See [CODING_RULES.md](CODING_RULES.md) for coding standards.

## Performance

Tested on Windows 11, Intel Core i7, 1 Gbps connection:

| Config | Speed | Notes |
|--------|-------|-------|
| Baseline (8KB, single) | ~360 KB/s | 1x |
| Power-of-two chunking | **10.5 MB/s** | **37x** (590 MB in 56s) |

Memory usage stays under 20 MB regardless of video size (streaming I/O).

## Legal

We only support sites with permissive Terms of Service. See [CONTRIBUTING.md](CONTRIBUTING.md) for site selection criteria.

Users are responsible for complying with applicable laws and respecting website ToS.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
