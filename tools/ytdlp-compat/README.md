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

## Slice-2 surface (SVT support — 2026-04-30)

Helper additions verified against yt-dlp tag `2026.03.17`:

- **Utility helpers** (`_utils.py`): `determine_ext`, `dict_get` (with `skip_false_values=`), `require`, `variadic`. Plus `RequiredError` exception subclass for `traverse_obj` propagation.
- **`traverse_obj` rewrite** (`info_extractor.py`): now supports `{type}` / `{type, type, ...}` / `{callable}` set-syntax segments, `(branch_a, branch_b)` tuple sub-paths, two-arg `lambda k, v: ...` predicates, and `any` / `all` / `filter` builtin terminators. Drops upstream support for `re.Match`, `xml.etree`, `http.cookies.Morsel`, `slice`, and `traverse_string` — none exercised by SVT or any plausible Slice-2 plugin. Add when a port needs them.
- **`InfoExtractor` methods**: `_match_valid_url`, `_match_id`, `suitable`, `ie_key` (classmethods); `url_result`, `playlist_result` (statics); `_og_search_title`, `_og_search_thumbnail`, `_og_search_property`; `_search_json` (regex + brace-balanced + parse_json), `_search_nextjs_data`; `_merge_subtitles` (+ `_merge_subtitle_items` dedupe); `_download_json`; `geo_verification_headers` (returns `{}` since shim has no YoutubeDL params surface). `_extract_f4m_formats` is a stub (F4M is dead).
- **Multi-class plugin support** (`_dispatch.py`): `discover_ie_classes` + `dispatch_url` honour SVT-style sibling `suitable()` overrides. Build-from-ytdlp's `extract_valid_urls` (Rust) captures every `_VALID_URL` (single + triple-quoted), skips docstring examples, and emits a deduped match-pattern union into the manifest.

## Limitations (deferred to Slice 2.5)

- Pure-data helpers (`traverse_obj`, `dict_get`, `int_or_none`, …) MUST stay Python — they take Python callables / type objects which can't cross the WIT boundary. Slice 2.5 will move I/O helpers (`_extract_m3u8_formats_and_subtitles`, `_search_regex`, etc.) host-side via a v0.2 WIT bump. See memory `project_ytdlp-shim-slice2_5-host-helpers`.
- `_parse_json` filters yt-dlp's `LenientJSONDecoder`-only kwargs (`ignore_extra` / `strict` / `lenient`) before forwarding to stdlib `json.loads`. Lenient parsing semantics (trailing data, control chars) are NOT supported. SVT's payloads are well-formed.
- The Next.js / `urqlState` extraction path needs a hydrated HTML fixture to test (the SSR snapshot lacks `urqlState`). The svt:short-form test URL exercises everything else end-to-end.

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

## Slice-2.5 surface (host-side I/O helpers — 2026-04-30)

Helpers moved host-side in v0.2 of the WIT contract. Plugin authors see
no change in method signatures — `InfoExtractor` method bodies became
2-line passthroughs over the new `host:extract-helpers` capability.
The wasm artefact shrinks because the regex/HTML/m3u8 parsing libraries
no longer ship per plugin.

**Drop-in workflow (post Slice 2.5):**

```bash
cp $YT_DLP_TREE/yt_dlp/extractor/foo.py examples/plugins/foo/foo.py
rdlp plugin build-from-ytdlp examples/plugins/foo/foo.py
bash scripts/sign-plugin.sh foo/plugin.wasm foo/plugin.toml.template > plugin.toml
cp foo/plugin.wasm plugin.toml ~/.config/rdlp/plugins/foo/
```

No source edits to `foo.py`. The fake `yt_dlp/` package staged at
build time resolves all upstream relative imports.

**WIT v0.2 host helpers:**

- `search-regex`, `html-search-regex`, `html-search-meta` — regex / OG / meta primitives
- `og-search-property` — OG property + entity unescape
- `extract-m3u8` — HLS master playlist parsing (lossless dict round-trip)
- `extract-mpd` — DASH MPD manifest parsing with segment extraction; subtitles slot now populated from text AdaptationSet sidecar tracks (fragmented text tracks deferred — log-warn + skip)
- `extract-json-ld` — typed JSON-LD video extraction backed by rdlp's existing parser
- `rta-search` — adult-content age-marker scan
- `search-json` — brace-balanced JSON extraction (Next.js / urqlState)

**Stays Python-resident:**

- `traverse_obj` — path segments contain Python type objects (`{str}`,
  `{dict}`) and closures (`lambda _, v: ...`, `{require('id')}`); these
  cannot cross the Component Model boundary.
- All pure-data helpers (`int_or_none`, `dict_get`, `parse_duration`,
  `url_or_none`, `str_to_int`, `merge_dicts`, `unified_strdate`,
  `format_field`, etc.).
- `InfoExtractor` class scaffolding + `raise_*` helpers.
