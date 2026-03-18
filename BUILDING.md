# Building from Source

## Requirements

- **Rust 1.85+** (2024 edition)
- **FFmpeg 5-8 shared libraries** (headers and `.lib`/`.so`/`.dylib`)
- **clang/libclang** — needed by `bindgen` to generate FFmpeg FFI bindings
- **C compiler** (MSVC on Windows, gcc on Linux, clang on macOS) — needed to compile bundled SQLite

TLS is handled by Rustls. No OpenSSL or system TLS libraries are required.

## FFmpeg

The build links dynamically against FFmpeg. The libraries must be installed
and discoverable before running `cargo build`.

### Linux

Install the development packages for your distribution:

```
# Debian/Ubuntu
apt install libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev libswscale-dev libswresample-dev libclang-dev

# Fedora
dnf install ffmpeg-devel clang-devel

# Arch
pacman -S ffmpeg clang
```

`pkg-config` must be able to find the FFmpeg libraries.

#### Custom FFmpeg build

If you have a custom FFmpeg build (e.g. with additional codecs), point
`PKG_CONFIG_PATH` at its pkgconfig directory. Create `.cargo/config.toml`
(gitignored) from the provided example:

```
cp .cargo/config.toml.example .cargo/config.toml
```

Then edit it to set your FFmpeg pkgconfig path. If your custom build bundles
its own copies of system libraries (fontconfig, harfbuzz, freetype, etc.),
isolate the FFmpeg `.pc` files into a separate directory to avoid conflicts
with system GTK/GDK packages:

```bash
mkdir -p /path/to/ffmpeg-build/lib/pkgconfig-ffmpeg
ln -s /path/to/ffmpeg-build/lib/pkgconfig/libav*.pc /path/to/ffmpeg-build/lib/pkgconfig-ffmpeg/
ln -s /path/to/ffmpeg-build/lib/pkgconfig/libsw*.pc /path/to/ffmpeg-build/lib/pkgconfig-ffmpeg/
```

Then in `.cargo/config.toml`:

```toml
[env]
PKG_CONFIG_PATH = { value = "/path/to/ffmpeg-build/lib/pkgconfig-ffmpeg:/usr/lib/pkgconfig:/usr/share/pkgconfig", force = true }
```

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

## Faster Builds

The workspace ships with a tuned `[profile.dev]` in `Cargo.toml` that
reduces debug info and optimizes third-party dependencies. This is active
by default for all developers.

For an additional speedup on Windows, create `.cargo/config.toml` (gitignored)
and enable the bundled `rust-lld` linker:

```toml
[target.x86_64-pc-windows-msvc]
linker = "rust-lld"
```

On Linux, use `mold` instead (requires GCC 12+ or mold 2.0+):

```toml
[target.'cfg(target_os = "linux")']
rustflags = ["-C", "link-arg=-fuse-ld=mold"]
```

On macOS, the default linker is already fast; no override is needed.

With these settings, incremental rebuilds of a single crate take 2–6 seconds
instead of ~50 seconds.

## Troubleshooting

**`Cannot find clang` / `libclang.so not found`**

Install clang/libclang. On Arch: `pacman -S clang`. On Debian/Ubuntu:
`apt install libclang-dev`. Or set `LIBCLANG_PATH` to the directory
containing `libclang.so`.

**FFmpeg not found (Windows)**

The build script prints warnings if it cannot locate FFmpeg. Set `FFMPEG_DIR`
to the root of your shared FFmpeg build and ensure it contains `include/` and
`lib/` subdirectories.

**Linker errors for avcodec/avformat/avutil**

FFmpeg shared libraries are not installed or not on the library search path.
On Linux, verify with `pkg-config --libs libavcodec`. On macOS, verify
Homebrew's FFmpeg is linked: `brew link ffmpeg`.

**`gdk-3.0` / `fontconfig` / `harfbuzz` version mismatch**

If using a custom FFmpeg build that bundles its own copies of system
libraries, their `.pc` files can shadow system versions and cause version
conflicts with GTK/GDK. See the "Custom FFmpeg build" section above for the
pkgconfig isolation workaround.

**`cc` crate fails to find a C compiler**

Bundled SQLite compilation requires a C compiler. On Windows, install the
Visual Studio Build Tools (MSVC). On Linux, install `build-essential` or
equivalent. On macOS, install Xcode Command Line Tools: `xcode-select --install`.
