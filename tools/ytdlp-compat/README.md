# rdlp_ytdlp_compat

Python compatibility shim that re-implements yt-dlp's `InfoExtractor` base class against rdlp's WASM host capabilities. Bundled into Python WASM plugins by `rdlp plugin build-from-ytdlp`.

## Pinned toolchain

componentize-py is pinned to **0.17.2** in `requirements-dev.txt`. This is the last release targeting wasmtime 30. Bumping requires upgrading `wasmtime` in `crates/rdlp-plugin/Cargo.toml` in lockstep — see `docs/planning/2026-04-29-ytdlp-compat-shim-slice-1.md`.

## Authoring constraint

componentize-py only resolves `import` / `from x import y` at module top level (issue #23). Lazy `__import__` and conditional imports silently fail. Hoist all imports.
