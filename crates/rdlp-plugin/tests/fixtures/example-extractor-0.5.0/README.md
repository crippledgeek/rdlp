# example-extractor-0.5.0 fixture

Compiled component for the ABI compat test suite (rdlp#762 slice B).

- **Source**: `examples/plugins/example-extractor` at commit `6d870b7ac716566a554032e9e9c8a42f9d6c62cd`
- **Built with**: plain `cargo build` (NOT `cargo-component build`) against
  `wasm32-unknown-unknown`, then composed into a component with
  `wasm-tools component new` (no `--adapt`) — see "Fix round 1" below for why.
- **Target**: `wasm32-unknown-unknown`, `--release`
- **WIT world targeted**: `example:extractor@0.1.0` (`example`), which `include`s
  `rdlp:plugin/extractor-plugin@0.5.0` — this is the world declared in
  `wit/world.wit`, but `wasm-tools component new` (this build path) does NOT
  preserve that package/world name in the composed artifact: running
  `wasm-tools component wit plugin.wasm` on the checked-in file reports
  `package root:component; world root { ... }` instead. The imports/exports
  are identical either way (only `rdlp:plugin/types@0.5.0` imported, the
  three `metadata`/`extract`/`search` exports) — only the name is renamed to
  the generic `root:component`/`root`, so don't expect the live dump to echo
  `example:extractor@0.1.0`/`example` back.
- **Size**: 39394 bytes (38.5 KiB)
- **sha256**: `8faa4a92aad481e38479d7ba5f672cad7ebaf8397533d27a116a4949f78fcf47`
- **No WASI imports** — loadable by the production loader
  (`PluginExtractor::new` → `host::add_capability_imports`, which wires only
  the manifest's declared rdlp capabilities and never WASI).

Built against `rdlp:plugin@0.5.0` BEFORE the 0.5.1 bump; it is the positive
compat fixture for `loader::tests::a_0_5_0_component_loads_on_the_0_5_1_host`.

## Fix round 1 — why this is NOT a `cargo-component build` artifact

The original fixture (this same source, built with `cargo component build
--release --target wasm32-wasip1`) declared `wasi:cli/environment@0.2.3` and
similar WASI 0.2 imports at the component root, because `example-extractor` is
a `std` crate and `cargo-component`'s `wasm32-wasip1` path always fuses in the
wasip1→preview2 adapter regardless of whether the plugin code actually calls
any WASI-backed function. `rdlp-plugin`'s production loader wires ONLY the six
rdlp capability interfaces (see `crates/rdlp-plugin/src/lib.rs` module docs,
"Known limitations") and never WASI — so that fixture could never actually be
loaded by the real loader; it would trap at instantiation with `component
imports instance 'wasi:cli/environment@0.2.3' ... not found in the linker`,
making it useless as Task 2's "loads through the real loader" positive case.

Rebuild the WASI-free component with:

```bash
rustup target add wasm32-unknown-unknown   # if not already installed
cd examples/plugins/example-extractor
cargo build --release --target wasm32-unknown-unknown
wasm-tools component new \
  target/wasm32-unknown-unknown/release/example_extractor.wasm \
  -o plugin.wasm
cp plugin.wasm ../../../crates/rdlp-plugin/tests/fixtures/example-extractor-0.5.0/plugin.wasm
```

`wasm-tools component wit plugin.wasm` on the result shows exactly one world
import — `rdlp:plugin/types@0.5.0` — no WASI of any kind. Verified against the
production loader path (`instance::build_store` + an empty
`Linker<PluginStoreData>`, matching `example-extractor`'s
`capabilities = []`): the component instantiates on the `extractor-plugin-host`
world (Fact C) and fails with `no function export `search-filters` found`
against the full `extractor-plugin` world (Fact D) — both re-confirmed with
this rebuild; see `docs/superpowers/reports/2026-09-15-since-annotation-acceptance.md`
"Fix round 1" section for full command+output transcripts.
