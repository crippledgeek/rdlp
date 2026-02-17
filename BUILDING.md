# Building from Source

## Requirements

- **Rust 1.85+** (2024 edition)
- **FFmpeg 5-8 shared libraries** (headers and `.lib`/`.so`/`.dylib`)
- **C compiler** (MSVC on Windows, gcc on Linux, clang on macOS) — needed to compile bundled SQLite

TLS is handled by Rustls. No OpenSSL or system TLS libraries are required.

## FFmpeg

The build links dynamically against FFmpeg. The libraries must be installed
and discoverable before running `cargo build`.

### Linux

Install the development packages for your distribution:

```
# Debian/Ubuntu
apt install libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev libswscale-dev libswresample-dev

# Fedora
dnf install ffmpeg-devel

# Arch
pacman -S ffmpeg
```

`pkg-config` must be able to find the FFmpeg libraries.

### macOS

```
brew install ffmpeg
```

### Windows

Install a shared FFmpeg build. The build script auto-detects these locations
in order:

1. `FFMPEG_DIR` environment variable (highest priority)
2. WinGet package (`Gyan.FFmpeg.Shared`)
3. Chocolatey (`ffmpeg-shared` or `ffmpeg` package)
4. Program Files directories
5. Derived from `ffmpeg.exe` in `PATH`

The directory must contain `include/libavcodec/` and `lib/` subdirectories.

To set explicitly:

```
set FFMPEG_DIR=C:\path\to\ffmpeg-shared
```

Or install via WinGet:

```
winget install Gyan.FFmpeg.Shared
```

## Clone

```
git clone https://github.com/crippledgeek/rdlp.git
cd rdlp
```

## Build

```
cargo build --release
```

The binary is at `target/release/rdlp` (`rdlp.exe` on Windows).

Debug build:

```
cargo build
```

## Run

```
./target/release/rdlp --help
```

## Tests

```
cargo test
```

Single crate:

```
cargo test -p rdlp-extractor
```

Lint and format checks:

```
cargo clippy -- -D warnings
cargo fmt --check
```

## Troubleshooting

**FFmpeg not found (Windows)**

The build script prints warnings if it cannot locate FFmpeg. Set `FFMPEG_DIR`
to the root of your shared FFmpeg build and ensure it contains `include/` and
`lib/` subdirectories.

**Linker errors for avcodec/avformat/avutil**

FFmpeg shared libraries are not installed or not on the library search path.
On Linux, verify with `pkg-config --libs libavcodec`. On macOS, verify
Homebrew's FFmpeg is linked: `brew link ffmpeg`.

**`cc` crate fails to find a C compiler**

Bundled SQLite compilation requires a C compiler. On Windows, install the
Visual Studio Build Tools (MSVC). On Linux, install `build-essential` or
equivalent. On macOS, install Xcode Command Line Tools: `xcode-select --install`.
