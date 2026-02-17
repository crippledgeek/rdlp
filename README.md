# rdlp

A video downloader that extracts metadata and media from supported
sites, downloads via HTTP or HLS with parallel chunking, and
post-processes with FFmpeg library bindings. Inspired by
[yt-dlp](https://github.com/yt-dlp/yt-dlp).

## Features

- HTTP and HLS downloads with resume support
- Parallel chunked transfers
- FFmpeg-based post-processing (remux, transcode, audio extraction, metadata/thumbnail embedding)
- yt-dlp-compatible format selection and output templates
- Browser cookie extraction (Chrome, Firefox) and Netscape cookie files
- Rate limiting, download archive, JSON metadata export
- Interactive format and container selection

## Supported Sites

PornHub, XHamster, RedTube, XTits, TNAFlix, EMPFlix, MovieFap.

See `rdlp --list-extractors` for the current list.

## Building

Requires Rust 1.85+ and FFmpeg shared libraries.

```
git clone https://github.com/crippledgeek/rdlp.git
cd rdlp
cargo build --release
```

See [BUILDING.md](BUILDING.md) for platform-specific FFmpeg setup and troubleshooting.

## Usage

```
rdlp URL                              # download best quality
rdlp -i URL                           # interactive format selection
rdlp --remux=mp4 URL                  # remux to MP4
rdlp -f "bv[height<=720]+ba" URL      # format selection
rdlp -x --audio-format=flac URL       # extract audio
rdlp --cookies-from-browser chrome URL # use browser cookies
rdlp --dump-json URL                  # metadata as JSON
```

Run `rdlp --help` for the full option list.

## Architecture

16-crate Cargo workspace. Three-stage pipeline: extract, download, post-process.

| Crate | Purpose |
|-------|---------|
| `rdlp-types` | Data types (Config, Format, InfoDict) |
| `rdlp-core` | Traits and error types |
| `rdlp-api` | Frontend-agnostic download engine |
| `rdlp-security` | SSRF protection, URL validation |
| `rdlp-http` | HTTP client factory |
| `rdlp-ratelimit` | Token-bucket rate limiter |
| `rdlp-crypto` | URL decryption |
| `rdlp-extractor` | Site extractors |
| `rdlp-downloader` | HTTP + HLS downloaders |
| `rdlp-ffmpeg` | FFmpeg library bindings |
| `rdlp-postprocess` | Post-processing pipeline |
| `rdlp-cookies` | Browser cookie extraction |
| `rdlp-jsinterp` | JavaScript interpreter |
| `rdlp-plugin` | Plugin system |
| `rdlp-table` | Format selection table layout |
| `rdlp-cli` | CLI application |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and [CODING_RULES.md](CODING_RULES.md).

## License

MIT. See [LICENSE](LICENSE).
