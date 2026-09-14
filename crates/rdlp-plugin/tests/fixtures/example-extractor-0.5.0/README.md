# example-extractor-0.5.0 fixture

Compiled component for the ABI compat test suite (rdlp#762 slice B).

- **Source**: `examples/plugins/example-extractor` at commit `6d870b7ac716566a554032e9e9c8a42f9d6c62cd`
- **Built with**: `cargo-component-component 0.21.1`
- **Target**: `wasm32-wasip1`, `--release`
- **WIT world targeted**: `example:extractor@0.1.0` (`example`), which `include`s
  `rdlp:plugin/extractor-plugin@0.5.0`
- **Size**: 90718 bytes (90.7 KiB)
- **sha256**: `393fb85efc3c9beee7998c861472261da0259b45bd68389c09b3bc2ad2ab4440`

Built against `rdlp:plugin@0.5.0` BEFORE the 0.5.1 bump; it is the positive
compat fixture for `loader::tests::a_0_5_0_component_loads_on_the_0_5_1_host`.

Rebuild from scratch with:

```bash
cd examples/plugins/example-extractor
cargo component build --release
cp target/wasm32-wasip1/release/example_extractor.wasm \
  ../../../crates/rdlp-plugin/tests/fixtures/example-extractor-0.5.0/plugin.wasm
```
