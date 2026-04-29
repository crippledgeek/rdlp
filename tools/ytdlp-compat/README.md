# rdlp_ytdlp_compat

Python compatibility shim that re-implements yt-dlp's `InfoExtractor` base class against rdlp's WASM host capabilities. Bundled into Python WASM plugins by `rdlp plugin build-from-ytdlp`.

## Pinned toolchain

`componentize-py-pin@0.17.2` — last release targeting wasmtime 30 (rdlp's host pin). Bumping requires upgrading `wasmtime` in `crates/rdlp-plugin/Cargo.toml` in lockstep — see `docs/planning/2026-04-29-ytdlp-compat-shim-slice-1.md`. Search the repo for the marker `componentize-py-pin@0.17.2` to find every version-specific workaround that needs revisiting on upgrade.

`requirements-dev.txt` is hash-pinned. Install with:

```bash
# Recommended — uv (Astral) is ~10x faster and ships with caching:
uv venv .venv --python 3.12
uv pip install --python .venv/bin/python --require-hashes -r requirements-dev.txt

# pip equivalent if you don't have uv:
python3 -m venv .venv
.venv/bin/pip install --require-hashes -r requirements-dev.txt
```

Both `uv pip` and `pip` refuse to install if any artifact's SHA-256 doesn't match — protecting against PyPI mirror compromise or a substituted-bytes attack against the pinned version.

## Authoring constraint

componentize-py only resolves `import` / `from x import y` at module top level (issue #23). Lazy `__import__` and conditional imports silently fail. Hoist all imports.

## Drop-in compatibility with `from yt_dlp.utils import ...`

The package's exception hierarchy mirrors yt-dlp's exactly so ported extractor source can be substituted with no code changes. `ExtractorError`, `UnsupportedError`, `RegexNotFoundError`, `GeoRestrictedError`, `UserNotLive`, `DownloadError`, `UnavailableVideoError`, `ContentTooShortError`, `PostProcessingError`, `DownloadCancelled`, and `YoutubeDLError` all match upstream's class names and constructor signatures (see `_errors.py` for the upstream-citation table).

## Regenerating requirements-dev.txt

When bumping `componentize-py` (or any dep) and re-pinning hashes:

```bash
cd tools/ytdlp-compat
python3 -m venv /tmp/regen-venv && . /tmp/regen-venv/bin/activate
pip install --upgrade pip pip-tools
cat > /tmp/regen.in <<EOF
componentize-py==0.17.3   # new version
pytest==8.3.4
EOF
pip-compile --generate-hashes --strip-extras --output-file=requirements-dev.txt /tmp/regen.in
```

After regeneration:
1. Re-add the leading comment block (`pip-compile` strips it).
2. Re-add the Windows-only `colorama` block — `pip-compile` drops platform-conditional deps when run on Linux. Fetch the hash via `curl -s https://pypi.org/pypi/colorama/0.4.6/json | jq -r '.urls[] | "\(.filename)|\(.digests.sha256)"'`.
3. Verify on a clean venv: `pip install --require-hashes -r requirements-dev.txt`.
4. Update the `componentize-py-pin@<version>` markers throughout the repo (grep for the previous version string).
