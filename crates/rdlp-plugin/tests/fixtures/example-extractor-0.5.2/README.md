# example-extractor-0.5.2 fixture

Compiled component for the ABI compat test suite (rdlp#762 slice C0-a,
issue #768): the same `examples/plugins/example-extractor` source as the
0.5.0 fixture, rebuilt against `rdlp:plugin@0.5.2` so it exports the three
post-0.5.0 functions the host resolves by name (`search-filters`,
`extract-playlist`, `extract-with-metadata`).

- **Source**: `examples/plugins/example-extractor` at commit `61701a214f10ec6be5a9a1411a2a7cb61a2c96a6`
- **Built with**: plain `cargo build` (NOT `cargo-component build`) against
  `wasm32-unknown-unknown`, then composed into a component with
  `wasm-tools component new` (no `--adapt`) — the 0.5.0 README's "Fix round 1"
  explains why the `cargo-component`/`wasm32-wasip1` path produces a
  component the production loader cannot instantiate (WASI imports the host
  never wires). `cargo component bindings` (cargo-component 0.21.1) first,
  because `src/bindings.rs` is gitignored generated code; wasm-tools 1.248.0.
- **Target**: `wasm32-unknown-unknown`, `--release`
- **WIT world targeted**: `example:extractor@0.1.0` (`example`), which `include`s
  `rdlp:plugin/extractor-plugin@0.5.2` — the world declared in `wit/world.wit`.
  As with the 0.5.0 fixture, `wasm-tools component new` does NOT preserve that
  package/world name in the composed artifact: `wasm-tools component wit
  plugin.wasm` on the checked-in file reports `package root:component; world
  root { ... }`. The imports/exports are identical either way; only the name
  is the generic `root:component`/`root`.
- **Size**: 65344 bytes (63.8 KiB)
- **sha256**: `f54e3aa39a689e618204e668783b07f21b0ea8a92e3738505bcb0cb91780ef79`
- **No WASI imports** — loadable by the production loader
  (`PluginExtractor::new` → `host::add_capability_imports`, which wires only
  the manifest's declared rdlp capabilities and never WASI).

Rebuild with:

```bash
rustup target add wasm32-unknown-unknown   # if not already installed
cd examples/plugins/example-extractor
cargo component bindings                   # regenerates the gitignored src/bindings.rs
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
  target/wasm32-unknown-unknown/release/example_extractor.wasm \
  -o ../../../crates/rdlp-plugin/tests/fixtures/example-extractor-0.5.2/plugin.wasm
```

The component's world, as `wasm-tools component wit plugin.wasm` prints it:

```
  import rdlp:plugin/types@0.5.2;
  export search-filters: func() -> list<search-filter-descriptor>;
  export extract-playlist: func(url: string, page: u32) -> result<playlist-page, playlist-error>;
  export extract-with-metadata: func(url: string) -> result<extraction, metadata-extract-error>;
  export metadata: func() -> plugin-info;
  export extract: func(url: string) -> result<info-dict, extract-error>;
  export search: func(query: search-query) -> result<search-page, search-error>;
```

One import, no WASI of any kind. The host still binds only the frozen
`extractor-plugin-host` world (`wit/COMPATIBILITY.md` §3); the three extra
exports are reached through `adapter::call_export_by_name`, which is exactly
what the tests below prove.

## What the fixture answers, by URL

The tests pin these; the table is the contract the source
(`examples/plugins/example-extractor/src/lib.rs`) implements.

| Export | URL | Answer |
|---|---|---|
| `extract` / `extract-with-metadata` | `https://example.com/video/<digits>` | `Example Video <digits>`; extra: `actors = ["Example Actor"]`, `age-limit = 18`, extras `studio = "Example Studio"`, `series = "Example Series"` |
| `extract-with-metadata` | `https://example.com/video/unsupported` | `err(unsupported-url)` (a domain outcome, no strike) |
| `extract` | `https://example.com/not-a-playlist` | a single video with id `not-a-playlist` — the target of the playlist fallback |
| `extract-playlist` | `https://example.com/a-real-playlist` | page 1: videos 1, 2 (`has-more = true`); page 2: video 3 (`has-more = false`) |
| `extract-playlist` | `https://example.com/internal-error-playlist` | `err(internal)` — the one playlist error that strikes |
| `extract-playlist` | `https://example.com/gone-playlist` | `err(not-found)` — propagated, not a fallback |
| `extract-playlist` | any other URL (incl. `not-a-playlist`) | `err(unsupported-url)` — the host falls back to one `extract` |

`extract` is implemented as `extract-with-metadata(url).core`, as the WIT
contract asks of a plugin exporting both.

## Tests that load it

- `tests/abi_0_5_2_fixture.rs` — through the production loader: the metadata
  lift, the `Config` extras cap, the domain-error mapping, the two-page
  playlist, and the refusals (unsigned / tampered wasm / tampered manifest /
  `wit_version = "0.5.3"`), the last four run over this fixture AND the 0.5.0
  one through one helper.
- `src/playlist_adapter/tests.rs` — the probe-then-fall-back path of
  `PluginExtractor::extract_playlist` by URL (fallback, strike, propagate, real
  listing fetched once).

Bytes are exposed once as `test_support::EXAMPLE_0_5_2_WASM`.
