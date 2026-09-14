# WIT contract compatibility policy

Governs `rdlp:plugin` version bumps across `types.wit`, `host.wit`,
`extractor.wit`. Applies to every future change to this package, not just
0.5.1.

## 1. Version rule

The Component Model Explainer's "Canonical Interface Name" folds a `0.x.y`
package version to `0.x` — `rdlp:plugin@0.5.0` and `@0.5.1` are the *same*
canonical name and link to each other; `@0.6.0` is a different name and
orphans every 0.5.x plugin. So: **patch = additive** (new functions, new
records, new exports with stubs); **minor = breaking**, accepted to orphan
older plugins; **never ship a pre-release tag** (`0.5.1-rc1` canonicalizes to
its own distinct name, not into `0.5`). Host check
(`loader::check_wit_version_against`): `plugin.major == host.major &&
plugin.minor == host.minor && plugin.patch <= host.patch`.

## 2. What "additive" means here

Never add a field to a shipped record; never change an existing function's
signature. Add a new record or a new function instead. `format`, `info-dict`,
`plugin-info`, `search-query`, `search-page` are frozen as of 0.5.0 — a
patch bump must not touch their fields.

## 3. Exports added after 0.5.0 are optional to the host

`wasmtime-wit-bindgen` 30 requires every world-level export to exist at
`instantiate` time — Task 1's measurement reproduced the exact failure
against a 0.5.0-built component bound to the full `extractor-plugin` world:
`no function export `search-filters` found`. `extractor.wit` splits the
contract in two: `extractor-plugin-host` (the frozen 0.5.0 exports, which
`lib.rs` binds) and `extractor-plugin` (`include`s the host world, adds
`@since(version = 0.5.1) search-filters`). The host binds only the smaller
world at compile time and looks up later exports by name at call time,
falling back to a stub (`search-filters` → `[]`) when absent — this is what
lets a 0.5.0 component instantiate on a 0.5.1 host at all.

## 4. `@since` toolchain acceptance (measured 2026-09-15)

`cargo-component 0.21.1`: accepted — builds cleanly once the new export is
implemented, no warning. `componentize-py 0.17.2`: accepted — bindings and
componentize steps both exit 0, no warning. Full transcripts:
`docs/superpowers/reports/2026-09-15-since-annotation-acceptance.md`.

## 5. What does NOT belong in the host surface

Site-specific obfuscation. XHamster's PRNG-based URL decryption stays
plugin-side, linked as the `rdlp-crypto` crate (no I/O, wasm-clean) rather
than a host import — a host import would freeze one site's decryption
scheme into every future ABI version, for every plugin, forever.

## 6. Deferred to the next minor

Splitting `extract` and `search` into separate WIT interfaces, so a plugin
exports only the capability it provides instead of a stubbed no-op. This
renames exports (a breaking change) and is deferred to the next minor bump.

## 7. Manifest fields (0.5.1 adds; Task 10 implements)

0.5.1 adds two `rdlp-plugin-manifest` TOML fields, not WIT records (adding a
field to `plugin-info` itself would be a breaking record change):
`supports_extract` (default `true`) and `search_site` (optional). Canonical
manifest bytes include `supports_extract` only when `false`, and
`search_site` only when present — so a 0.5.0 manifest's canonical bytes, and
its signature over them, stay valid unchanged. Implemented in Task 10 of
this slice.
