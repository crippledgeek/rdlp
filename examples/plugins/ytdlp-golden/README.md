# yt-dlp Golden Corpus

Three synthetic yt-dlp-shape extractors that exercise the `rdlp_ytdlp_compat` helper surface through the `rdlp plugin build-from-ytdlp` CLI pipeline. **These are not real yt-dlp extractors** — the URLs and field shapes are fictional. They exist to prove the build pipeline produces signed-able `.wasm` plus a schema-correct `plugin.toml.template` for realistic plugin authoring patterns.

| File | Helpers exercised |
|---|---|
| `simple_html.py` | `_download_webpage`, `_html_search_meta`, `_search_regex` |
| `json_traversal.py` | `_download_webpage`, `_parse_json`, `traverse_obj`, `int_or_none`, `try_get` |
| `m3u8_with_fallback.py` | `_download_webpage`, `_extract_m3u8_formats`, `urljoin`, `unified_timestamp` |

## Building

```bash
rdlp plugin build-from-ytdlp examples/plugins/ytdlp-golden/simple_html.py
```

Produces `simple_html/plugin.wasm` (~35 MB) and `simple_html/plugin.toml.template` next to the source.

## Running the integration test

```bash
cargo test -p rdlp-plugin --test ytdlp_golden -- --ignored --nocapture
```

The test builds all three extractors via the CLI and asserts on the produced artefacts. **Extract dispatch (executing the plugin against canned HTML) is deferred to Slice 2** — that requires a host-side fixture-injection harness that mocks `host:fetch`.

## Slice-2 follow-up

- Real yt-dlp upstream extractors adapted with proper fixture infrastructure.
- Fixture-injection harness in the host that mocks `host:fetch` against canned HTML for end-to-end extract testing.
