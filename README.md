# rdlp

**Rust Download Program** - A fast, extensible video downloader written in Rust.

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)](https://github.com/crippledgeek/rdlp)
[![Rust Version](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

Inspired by [yt-dlp](https://github.com/yt-dlp/yt-dlp). Built on tokio, reqwest, and FFmpeg library bindings.

## Features

- **Performance** - 10+ MB/s downloads with parallel chunking and power-of-two sizing
- **HLS support** - Full streaming with duration-based progress, DRM detection, live stream warnings, resume validation, PTS normalization
- **Post-processing** - FFmpeg library bindings for remux, audio extraction, video conversion, metadata embedding
- **Thumbnails** - Auto-download and embed cover art; MP4 uses iTunes `covr` atom for Windows Explorer visibility
- **28 container formats** - MP4, MKV, WebM, MOV, AVI, TS, FLV, 3GP, MPG, ASF/WMV, MXF, VOB, IVF, and audio containers (MP3, FLAC, WAV, Opus, AAC, AIFF, etc.)
- **16 video codecs** - H.264, H.265, VP8, VP9, AV1, VVC/H.266, MPEG-1/2/4, ProRes, DNxHD, Theora, FFV1, Xvid, WMV2
- **14 audio codecs** - MP3, AAC, M4A, Opus, Vorbis, FLAC, ALAC, WAV, AC-3, E-AC-3, DTS, MP2, WavPack, TTA
- **Resume** - Ctrl+C to pause, re-run to resume automatically
- **Format selection** - yt-dlp-compatible DSL with filters, merge (`+`), and fallback chains (`/`)
- **Interactive** - Arrow-key format selection, container remux menu
- **Playlists** - Batch downloads with pagination (PornHub)
- **Cookie support** - Browser extraction (Chrome, Firefox) and Netscape cookie files
- **Rate limiting** - Global bandwidth throttle with human-readable rates (`1M`, `500K`, `2.5G`)
- **Download archive** - Skip already-downloaded videos on re-run (`--download-archive`)
- **JSON metadata** - `--dump-json` and `--print` for scripting (`rdlp --dump-json URL | jq .title`)

### Supported Sites

| Site | Formats | Features |
|------|---------|----------|
| PornHub | HLS + MP4 | Multi-quality (240p-1080p), playlist support, CDN Referer auth |
| XHamster | HLS + MP4 | Multi-quality (144p-2160p), AV1 + H.264, encrypted URL decryption, user playlists |
| RedTube | HLS + MP4 | Multi-quality (240p-1080p), segment-based progress |
| XTits | MP4 | Direct download, multi-quality (480p-720p) |
| TNAFlix | MP4 | Multi-quality (144p-480p) |
| EMPFlix | MP4 | Multi-quality (144p-1080p) |
| MovieFap | MP4 | Multi-quality (144p-1080p) |

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

# Limit download speed
rdlp -r 1M "https://www.redtube.com/12345678"
rdlp --limit-rate 500K "https://www.redtube.com/12345678"

# Extract audio (interactive format selection)
rdlp -x --audio-format "https://www.redtube.com/12345678"
rdlp -x --audio-format=flac "https://www.redtube.com/12345678"

# Re-encode video (interactive codec selection)
rdlp --recode-video "https://www.redtube.com/12345678"
rdlp --recode-video=webm "https://www.redtube.com/12345678"

# List supported codecs
rdlp --list-codecs

# Thumbnail control
rdlp --no-thumbnail "https://www.redtube.com/12345678"       # Skip thumbnail download/embed
rdlp --write-thumbnail "https://www.redtube.com/12345678"     # Keep thumbnail file on disk

# Skip already-downloaded videos (playlist or repeated runs)
rdlp --download-archive archive.txt "https://www.pornhub.com/playlist/123456"

# Dump metadata as JSON (no download)
rdlp --dump-json "https://www.redtube.com/12345678"
rdlp --dump-json "https://www.redtube.com/12345678" | jq .title

# Print specific fields
rdlp --print title "https://www.redtube.com/12345678"
rdlp --print "id,title,extractor" "https://www.redtube.com/12345678"

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

### CLI Reference

```
rdlp [OPTIONS] [URL]
```

#### General

| Flag | Short | Description |
|------|-------|-------------|
| `--output <DIR>` | `-o` | Output directory (default: `.`) |
| `--format <FMT>` | `-f` | Format selection, yt-dlp syntax (default: `best`) |
| `--interactive` | `-i` | Interactive format selection with arrow keys |
| `--dump-json` | `-j` | Dump full metadata as JSON to stdout (no download) |
| `--print <FIELDS>` | | Print specific metadata field(s), comma-separated (no download) |
| `--simulate` | `-s` | Simulate only, show metadata summary |
| `--quiet` | `-q` | Minimal output |
| `--verbose` | `-v` | Detailed debug output |
| `--list-extractors` | | List all supported site extractors |
| `--list-downloaders` | | List all supported download protocols |
| `--list-codecs` | | List all supported audio and video codecs |

#### Network

| Flag | Short | Description |
|------|-------|-------------|
| `--limit-rate <RATE>` | `-r` | Limit download speed (e.g., `1M`, `500K`, `2.5G`) |
| `--proxy <URL>` | | HTTP/HTTPS/SOCKS proxy (e.g., `socks5://127.0.0.1:1080`) |

Rate limit supports binary unit suffixes: `K` (1024), `M` (1048576), `G` (1073741824). Decimal values work (e.g., `2.5M`). Plain numbers are bytes/s.

#### Post-processing

| Flag | Short | Description |
|------|-------|-------------|
| `--extract-audio` | `-x` | Extract audio only (requires FFmpeg) |
| `--audio-format[=FMT]` | | Audio format (14 codecs). Use `--audio-format` for interactive, `--audio-format=mp3` for direct |
| `--audio-quality <Q>` | | VBR level 0-9 or bitrate like `192K` |
| `--embed-metadata` | | Embed title, artist, etc. in the file |
| `--embed-thumbnail` | | Embed thumbnail in the file (default: on). MP4 writes both `attached_pic` stream and iTunes `covr` atom |
| `--no-thumbnail` | | Disable automatic thumbnail download and embedding |
| `--write-thumbnail` | | Keep thumbnail image file on disk alongside media file |
| `--recode-video[=FMT]` | | Re-encode video (16 codecs). Use `--recode-video` for interactive, `--recode-video=mp4` for direct |
| `--remux[=FMT]` | | Remux to container without re-encoding (28 formats). Normalizes timestamps to start at 0. Use `--remux` for interactive, `--remux=mp4` for direct |
| `--keep-video` | | Keep original file after post-processing |
| `--ffmpeg-location <PATH>` | | Path to FFmpeg if not in PATH |

#### Cookies

| Flag | Description |
|------|-------------|
| `--cookies-from-browser <BROWSER>` | Load cookies from browser (`chrome`, `firefox`) |
| `--cookies <FILE>` | Load Netscape-format cookies file |

#### Download Archive

| Flag | Description |
|------|-------------|
| `--download-archive <FILE>` | Path to archive file (skip already-downloaded videos) |

Records each completed download as `{extractor} {id}` (one per line) in a plain text file. On subsequent runs, videos already in the archive are skipped. Compatible with yt-dlp's archive format. Blank lines and `#` comments are ignored.

```
# Example archive file
PornHub ph6abc123def
RedTube 12345678
TNAFlix 456789
```

#### Configuration

| Flag | Description |
|------|-------------|
| `--config-location <FILE>` | Path to config file (TOML format) |
| `--ignore-config` | Skip loading config file |

rdlp loads configuration from a TOML file at startup. CLI flags override config file values.

**Default location:**
- Windows: `%APPDATA%\rdlp\config.toml`
- Linux/macOS: `~/.config/rdlp/config.toml`

**Example config file:**

```toml
format = "bv[height<=720]+ba/b"
output_directory = "C:\\Videos"
proxy = "socks5://127.0.0.1:1080"
rate_limit = 5242880
embed_metadata = true
verbose = false
quiet = false
```

All fields are optional — missing fields use defaults. See `Config` struct in `crates/rdlp-types/src/config.rs` for all available fields.

### Resume

Press **Ctrl+C** during download to pause. Re-run the same command to resume from where you left off.

HLS downloads validate segment files on resume — missing or corrupted segments are automatically re-downloaded.

## Architecture

15-crate workspace with a three-stage pipeline: **Extract** -> **Download** -> **Post-process**.

```
rdlp/
├── crates/
│   ├── rdlp-types/        # Pure data types (Config, Format, InfoDict)
│   ├── rdlp-table/        # Column layout constants for format selection table
│   ├── rdlp-core/         # Traits (InfoExtractor, Downloader, PostProcessor)
│   ├── rdlp-security/     # SSRF protection, URL validation
│   ├── rdlp-http/         # HTTP client factory
│   ├── rdlp-ratelimit/    # Async token-bucket rate limiter
│   ├── rdlp-crypto/       # PRNG-based URL decryption (XHamster)
│   ├── rdlp-extractor/    # Site extractors
│   ├── rdlp-downloader/   # HTTP + HLS downloaders
│   ├── rdlp-ffmpeg/       # FFmpeg library bindings (probe, remux, transcode, metadata)
│   ├── rdlp-postprocess/  # Post-processing pipeline (processors, registry, mp4ameta)
│   ├── rdlp-cookies/      # Browser cookie extraction (Chrome, Firefox, Netscape)
│   ├── rdlp-jsinterp/     # JavaScript interpreter (boa)
│   ├── rdlp-plugin/       # Plugin system
│   └── rdlp-cli/          # CLI application and orchestrator
└── docs/                  # Documentation
```


## Development

```bash
cargo build --release      # Optimized build
cargo test                 # Run all tests (400+)
cargo clippy               # Lint check
cargo fmt                  # Format code
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for adding extractors, code style, and development workflow.
See [CODING_RULES.md](CODING_RULES.md) for coding standards.

## Legal

We only support sites with permissive Terms of Service. See [CONTRIBUTING.md](CONTRIBUTING.md) for site selection criteria.

Users are responsible for complying with applicable laws and respecting website ToS.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
