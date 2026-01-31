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
- **Format selection** - yt-dlp-compatible DSL with filters, merge (`+`), and fallback chains (`/`)
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

# Format selection (yt-dlp compatible syntax)
rdlp -f "bv[height<=720]+ba" "https://www.redtube.com/12345678"
rdlp -f "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]" "https://www.redtube.com/12345678"
rdlp -f "worst" "https://www.redtube.com/12345678"

# Verbose mode
rdlp -v "https://www.redtube.com/12345678"
```

### Format Selection

Supports yt-dlp-compatible format selection syntax with filters, merge, and fallback chains.

```
# Syntax
expression   = format_spec ( "/" format_spec )*     # fallback chain
format_spec  = selector ( "+" selector )?           # video+audio merge
selector     = base_name filter*                    # base with optional filters
filter       = "[" field operator value "]"
```

| Selector | Meaning |
|----------|---------|
| `best`, `b` | Best combined (video+audio) format |
| `worst`, `w` | Worst combined format |
| `bestvideo`, `bv` | Best video-only stream |
| `bestaudio`, `ba` | Best audio-only stream |
| `bv*`, `ba*` | Best video/audio, may include both |
| `worstvideo`, `worstaudio` | Worst video/audio-only |

**Filters**: `height`, `width`, `ext`, `vcodec`, `acodec`, `fps`, `tbr`, `vbr`, `abr`, `asr`, `filesize`, `protocol`, `format_id` with operators `=`, `!=`, `<`, `>`, `<=`, `>=`.

```bash
# Examples
rdlp -f "best"                          # Best combined format (default)
rdlp -f "bv+ba"                         # Best video + best audio (merge)
rdlp -f "bv[height<=720]+ba"            # 720p or lower + best audio
rdlp -f "ba[abr>=128]"                  # Best audio with bitrate >= 128kbps
rdlp -f "bv[ext=mp4]+ba[ext=m4a]/b"    # MP4 video + M4A audio, fallback to best
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
