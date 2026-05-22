# Building from Source

## Requirements

- **Rust 1.85+** (2024 edition)
- **C compiler toolchain** (MSVC on Windows, gcc on Linux, clang on macOS)
- **clang/libclang** — needed by `bindgen` to generate FFmpeg FFI bindings
- **cmake** + **Perl** + **NASM** — needed by `wreq`'s BoringSSL build script (Phase 2 TLS impersonation) and by `openssl-sys` when building OpenSSL from source on Windows
- **FFmpeg 5–8 shared libraries** (headers and `.lib`/`.so`/`.dylib`)

TLS is handled by **wreq + BoringSSL** (built in-tree from source on every fresh build) — gives every HTTP request a real-browser JA4/JA4H fingerprint. On Linux, `openssl-sys` is also linked to coexist with FFmpeg's OpenSSL refs. On Windows, `openssl-sys` is built `vendored` (compiled from source by Cargo, no external OpenSSL install required). On macOS, neither is needed at the workspace level.

The first cold build is slow (~10–20 min depending on platform) because BoringSSL + OpenSSL are compiled from source. Incremental rebuilds are seconds.

## Linux

### System packages

```bash
# Debian/Ubuntu (matches CI)
sudo apt install -y \
    libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev \
    libavdevice-dev libswscale-dev libswresample-dev \
    libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf \
    cmake perl libclang-dev musl-tools

# Fedora
sudo dnf install -y ffmpeg-devel clang-devel cmake perl-core webkit2gtk4.1-devel

# Arch
sudo pacman -S --needed ffmpeg clang cmake perl webkit2gtk-4.1
```

`pkgconf` (the modern pkg-config drop-in) must be able to find the FFmpeg libraries. Most distros ship pkgconf preinstalled; **do not install both `pkgconf` and `pkg-config` simultaneously** — they conflict via apt's `Breaks` clause.

### Custom FFmpeg build (e.g. mediaforge)

If you have a custom FFmpeg build with extra codecs (e.g. via [mediaforge](https://github.com/crippledgeek/mediaforge)), install it to **its own isolated prefix** — not into a shared location like `~/.local`. Conventional choices:

- `~/.local/mediaforge` (or `~/.local/<build-name>`)
- `~/opt/mediaforge`
- `/opt/mediaforge`

Custom FFmpeg builds typically bundle their own copies of transitive system libraries (fontconfig, harfbuzz, freetype, etc.) as static archives. If those libraries' `.pc` files land directly under `~/.local/lib/pkgconfig/`, they shadow system versions when pkg-config walks dep chains for unrelated crates — breaking GTK/GDK-dependent builds (`gdk-sys`, `glib-sys`) with version-mismatch errors like:

```
Package 'fontconfig' has version '2.15.0', required version is '>= 2.17.0'
```

Installing to an isolated prefix scopes the shadowing risk to consumers that explicitly opt in via `PKG_CONFIG_PATH`. Create `.cargo/config.toml` (gitignored) from the provided example:

```bash
cp .cargo/config.toml.example .cargo/config.toml
```

Then point at your isolated prefix:

```toml
[env]
PKG_CONFIG_PATH = { value = "/home/youruser/.local/mediaforge/lib/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig", force = true }
```

`rdlp-ffmpeg`'s `build.rs` verifies the first path actually contains `libavcodec.pc` and emits a `cargo:warning=` on every build if it doesn't — so a stale prefix path or a vanished install surfaces loudly instead of silently falling back to the distro FFmpeg.

#### Troubleshooting: GTK-dependent crates fail to compile

If `cargo build --workspace` (or anything that pulls `gdk-sys`/`glib-sys` via `rdlp-desktop`) fails with `fontconfig` / `harfbuzz` version-mismatch errors even with the isolated prefix above, your custom FFmpeg's pkgconfig dir is still shipping transitive .pc files that conflict with the system. The cleanest fix is upstream (have the custom FFmpeg build install only FFmpeg-ecosystem .pc files: `libav*.pc`, `libsw*.pc`, codec-specific files like `libfdk-aac.pc` / `libx264.pc` / `libopus.pc`). As a local workaround, manually remove the shadowers from the prefix's pkgconfig dir:

```bash
PKGDIR=/home/youruser/.local/mediaforge/lib/pkgconfig
for pc in fontconfig harfbuzz harfbuzz-subset freetype2 expat gnutls libpng libpng16 \
          libxml-2.0 fribidi gmp hogweed nettle bzip2 liblzma libbrotli{common,dec,enc}; do
  rm -f "$PKGDIR/$pc.pc"
done
```

The static `.a` archives must remain — `libavcodec.a` etc. still need them at link time. Only the `.pc` files (which leak shadowing into pkg-config's dep walk) should be removed.

#### TLS coexistence

If your custom FFmpeg links against OpenSSL (most do by default), the in-tree `wreq` build needs `prefix-symbols` to coexist — this is enabled automatically on Linux via `crates/rdlp-http/Cargo.toml`. If your custom FFmpeg links against GnuTLS instead (mediaforge's preferred recipe), you can drop the `openssl-sys` linker dep, but the workspace currently keeps it on Linux for the gyan.dev / distro FFmpeg case.

## macOS

```bash
brew install ffmpeg
```

Homebrew's FFmpeg is `--enable-gpl` by default and uses GnuTLS. No extra setup needed; `wreq`'s `prefix-symbols` is **not** enabled on macOS (it's documented Linux/Android-only by the wreq upstream and breaks with a `build_script_main_*` symbol mangling on macOS).

## Windows

Windows requires the most setup because BoringSSL + OpenSSL + bindgen all need a full C toolchain. Install in this order, then build from a **Developer PowerShell for VS 2022** (the regular PowerShell does not have MSVC's INCLUDE/LIB env activated, which causes `bindgen` to crash with `STATUS_ACCESS_VIOLATION` on FFmpeg headers).

### 1. Visual Studio 2022 Build Tools (MSVC + Win SDK)

Pick whichever package manager you already use:

```powershell
# Chocolatey
choco install -y visualstudio2022buildtools visualstudio2022-workload-vctools

# winget
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

# OR manual: download from https://visualstudio.microsoft.com/downloads/?q=build+tools
# Run the installer and pick the "Desktop development with C++" workload
```

This installs MSVC (`cl.exe`), the Windows SDK headers, and `cmake`. Reboot after install.

### 2. Strawberry Perl, NASM, 7zip

```powershell
# Chocolatey
choco install -y strawberryperl nasm 7zip

# winget
winget install StrawberryPerl.StrawberryPerl
winget install NASM.NASM
winget install 7zip.7zip
```

- **Strawberry Perl** runs OpenSSL's and BoringSSL's build-time codegen scripts (`perl Configure ...`)
- **NASM** assembles BoringSSL's hand-written x86_64 perf primitives
- **7zip** extracts the FFmpeg `.7z` archive in the next step

### 3. Rust (MSVC target)

```powershell
Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile rustup-init.exe
.\rustup-init.exe -y --default-toolchain stable --profile minimal
# Restart shell so cargo is on PATH
```

The MSVC target (`x86_64-pc-windows-msvc`) is the default and what we support. The GNU target (`x86_64-pc-windows-gnu`) is **not** supported — BoringSSL doesn't build cleanly under MinGW.

### 4. FFmpeg shared build (pinned to 8.0)

`ffmpeg-sys-the-third v4.1.0+ffmpeg-8.0` declares 8.0 bindings. FFmpeg 8.1 headers crash bindgen on Windows. **Use the pinned 8.0 archive:**

```powershell
$url = "https://github.com/GyanD/codexffmpeg/releases/download/8.0/ffmpeg-8.0-full_build-shared.7z"
Invoke-WebRequest -Uri $url -OutFile ffmpeg.7z
& 'C:\Program Files\7-Zip\7z.exe' x ffmpeg.7z -oC:\ffmpeg

# Set env vars (current shell + persisted)
$ffmpegDir = (Get-ChildItem C:\ffmpeg -Directory)[0].FullName
[Environment]::SetEnvironmentVariable("FFMPEG_DIR", $ffmpegDir, "User")
[Environment]::SetEnvironmentVariable("Path", "$ffmpegDir\bin;$([Environment]::GetEnvironmentVariable('Path','User'))", "User")
```

The directory must contain `include/libavcodec/`, `lib/avcodec.lib`, and `bin/avcodec-*.dll`. Don't use a static FFmpeg build — `ffmpeg-sys-the-third` expects shared libs.

### 5. Defender exclusions (strongly recommended)

Cold builds compile BoringSSL + OpenSSL + FFmpeg bindgen + 18 workspace crates. Defender real-time-scanning every intermediate `.rlib` and `.obj` makes the build take 3-5x longer and frequently produces spurious "blocked" prompts. Add exclusions:

```powershell
Add-MpPreference -ExclusionPath (Get-Location).Path
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.cargo"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.rustup"
```

### 6. Build

**Open `Developer PowerShell for VS 2022`** from the Start menu (NOT a regular PowerShell). This activates the MSVC environment so clang (invoked by `bindgen`) can find Windows SDK + STL headers. Without it, `ffmpeg-sys-the-third`'s build script crashes with `STATUS_ACCESS_VIOLATION (0xc0000005)`.

```powershell
cd path\to\rdlp
cargo build --release
```

Cold build: 15-25 min (BoringSSL + vendored OpenSSL + FFmpeg bindgen all compile from source on first run). Cached after.

Output: `target\release\rdlp.exe`. To run elsewhere, ship the FFmpeg `.dll` files alongside the `.exe`.

## Clone

```bash
git clone https://github.com/crippledgeek/rdlp.git
cd rdlp
```

## Build

```bash
cargo build --release
```

The binary is at `target/release/rdlp` (`rdlp.exe` on Windows).

Debug build:

```bash
cargo build
```

## Desktop app (Tauri)

The desktop crate at `crates/rdlp-desktop/` is a Tauri v2 app with a React/TypeScript frontend. It's built separately:

```bash
cd crates/rdlp-desktop

# Frontend deps + build
npm install
npx vite build

# Type check + tests
npx tsc --noEmit
npm test -- --run
npm test -- --typecheck --run

# Desktop crate compile + test
cd ../..
cargo build -p rdlp-desktop --release
cargo test -p rdlp-desktop
```

For dev mode:

```bash
cd crates/rdlp-desktop
npm run tauri dev
```

## Run

```bash
./target/release/rdlp --help
```

## Tests

```bash
cargo test
```

Single crate:

```bash
cargo test -p rdlp-extractor
```

Lint and format checks:

```bash
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

## Faster Builds

The workspace ships with a tuned `[profile.dev]` in `Cargo.toml` that reduces debug info and optimizes third-party dependencies. This is active by default for all developers.

For an additional speedup on Windows, create `.cargo/config.toml` (gitignored) and enable the bundled `rust-lld` linker:

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

With these settings, incremental rebuilds of a single crate take 2–6 seconds instead of ~50 seconds.

## Troubleshooting

### `Cannot find clang` / `libclang.so not found`

Install clang/libclang. On Arch: `pacman -S clang`. On Debian/Ubuntu: `apt install libclang-dev`. On Windows: comes with VS Build Tools' "Desktop development with C++" workload. Or set `LIBCLANG_PATH` to the directory containing `libclang.so` / `libclang.dll`.

### `STATUS_ACCESS_VIOLATION` building `ffmpeg-sys-the-third` on Windows

bindgen invokes clang to parse FFmpeg headers, but clang on Windows needs MSVC's `INCLUDE` and `LIB` env vars to find the Windows SDK + STL headers. Without them, parsing the transitive include chain segfaults.

**Fix:** open `Developer PowerShell for VS 2022` from the Start menu and run `cargo build` from there. Don't use a regular PowerShell or cmd.exe.

### `Could not find directory of OpenSSL installation` on Windows

If you see this from `openssl-sys`, your build isn't using the `vendored` feature that the workspace enables for Windows targets. Verify `crates/rdlp-http/Cargo.toml` contains:

```toml
[target.'cfg(target_os = "windows")'.dependencies]
openssl-sys = { version = "0.9", features = ["vendored"] }
```

If it's there but you still get the error, run `cargo clean -p openssl-sys` to force a rebuild with the right features.

### `error: failed to run custom build command for boring-sys`

Usually means BoringSSL's build couldn't find one of its required tools:
- **NASM**: `nasm --version` should print 2.x. If not on PATH, the choco/winget install needs a fresh shell.
- **Perl**: `perl --version` should print 5.x. Strawberry Perl puts it on PATH automatically.
- **MSVC**: `cl.exe` should be on PATH inside a Developer PowerShell. If you're in a regular PowerShell, MSVC isn't on PATH.

### `pkgconf : Breaks: pkg-config (>= 0.29-1)` on Ubuntu

The runner / fresh install ships `pkgconf` which provides the `pkg-config` interface. Remove the `pkg-config` package (`sudo apt remove pkg-config`) and use `pkgconf` exclusively. Don't try to install both.

### Vite build fails with `Transforming destructuring to the configured target environment`

The Vite config at `crates/rdlp-desktop/vite.config.ts` should set `target: "esnext"`. If you've patched it to `safari13`, `safari14`, or another older target, modern TanStack vendor chunks will fail esbuild transpile. Use `esnext` — Tauri's WebView2/WebKitGTK/WebKit all support modern syntax natively.

### Linker errors for `avcodec`/`avformat`/`avutil`

FFmpeg shared libraries are not installed or not on the library search path. On Linux, verify with `pkgconf --libs libavcodec`. On macOS, verify Homebrew's FFmpeg is linked: `brew link ffmpeg`. On Windows, ensure `FFMPEG_DIR` points at the FFmpeg root (containing `include/` and `lib/` subdirectories), and that the FFmpeg version matches what `ffmpeg-sys-the-third` declares (currently `ffmpeg-8.0`).

### Linker errors for OpenSSL on Linux (`undefined reference to SSL_*`)

Ensure `libssl-dev` (Debian/Ubuntu) / `openssl-devel` (Fedora) / `openssl` (Arch) is installed. The workspace links `-lssl -lcrypto` on Linux to satisfy FFmpeg's OpenSSL refs.

### Undefined symbols `build_script_main_BIO_clear_retry_flags` on macOS

`wreq`'s `prefix-symbols` feature is documented Linux/Android-only by upstream. If you've enabled it on macOS (e.g. by adding it to the workspace `wreq` features instead of the Linux-conditional rdlp-http target dep), you'll see this. Verify `crates/rdlp-http/Cargo.toml` only enables `prefix-symbols` under `target.'cfg(target_os = "linux")'.dependencies`.

### `gdk-3.0` / `fontconfig` / `harfbuzz` version mismatch

If using a custom FFmpeg build that bundles its own copies of system libraries, their `.pc` files can shadow system versions and cause version conflicts with GTK/GDK. See the "Custom FFmpeg build" section above for the pkgconfig isolation workaround.

### Defender blocking the Windows build

Add `~/.cargo/registry/`, `~/.rustup/`, and the rdlp source folder to Defender exclusions (see Windows step 5 above). Without exclusions, the cold build is 3-5x slower and may surface spurious "scan in progress" prompts.

### `cc` crate fails to find a C compiler

Bundled SQLite + several other native deps need a C compiler. On Windows, install the Visual Studio Build Tools (MSVC). On Linux, install `build-essential` or equivalent. On macOS, install Xcode Command Line Tools: `xcode-select --install`.
