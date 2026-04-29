//! # rdlp-plugin
//!
//! WASM-based plugin system for rdlp. Loads polyglot WebAssembly Component
//! Model plugins from the filesystem and registers them as
//! [`rdlp_core::InfoExtractor`] implementations alongside built-in extractors.
//!
//! ## Architecture
//!
//! Plugins are signed `.wasm` components targeting the `extractor-plugin`
//! WIT world (see `wit/extractor.wit`). Each plugin lives as a directory
//! containing:
//!
//! - `plugin.toml` — manifest: name, version, match patterns, capabilities,
//!   priority, signature
//! - `plugin.wasm` — the component artefact
//!
//! ## Loading pipeline
//!
//! 1. [`loader::Loader::discover`] scans configured directories.
//! 2. Each manifest is parsed + validated ([`manifest`]) — wrong priority,
//!    unknown capabilities, malformed regex are rejected here.
//! 3. The signature is verified ([`signature`]) — Ed25519 today, Sigstore
//!    keyless once Task 6b lands ([#213][issue-213]).
//! 4. Identity is matched against the [`trust_store`] — mismatch refused;
//!    new name fires a first-install [`prompt`]; capability creep across
//!    versions fires a re-confirm prompt.
//! 5. The component is compiled with [`engine::Engine`] (Wasmtime + epoch
//!    interruption + component model + async support).
//!
//! ## Per-call execution
//!
//! Each plugin invocation gets a fresh [`wasmtime::Store`] populated with the
//! capability contexts the manifest declared. The store is bounded by
//! `StoreLimits` (64 MB memory, 1 MB stack default) and an epoch deadline
//! (30 s extract / 60 s search default). Cancellation is propagated via
//! `tokio::select!` against the per-call `CancellationToken`.
//!
//! ## Capabilities (host imports — opt-in)
//!
//! - [`host::fetch`] — HTTPS via the shared `wreq` client (TLS impersonation
//!   preserved, URL security validation, body cap, cancel-aware)
//! - [`host::cookie_jar`] — scoped to the plugin's match-pattern hosts
//!   (Public Suffix List + manifest); A3 vector mitigation
//! - [`host::js_eval`] — boa via [`rdlp_jsinterp`]
//! - [`host::html_select`] — `scraper`-backed CSS selector helper
//! - [`host::log`] — forwarded to the host `log` crate with target
//!   `plugin::<name>`
//! - [`host::store_kv`] — sled-backed K/V, namespaced per plugin, 10 MB quota
//!
//! ## Dispatch & priority
//!
//! - URL → match-pattern trie ([`dispatch::MatchTrie`]) returns candidate set
//! - Effective priority computed per URL ([`priority::effective_priority`])
//!   — plugins shadowing a built-in's host space without explicit
//!   `claims_override` are clamped to 99 so built-ins always win for their
//!   declared territory
//!
//! ## Trust model
//!
//! Four-layer hybrid (see design spec §9 for rationale):
//!
//! 1. **Mandatory signing** — Sigstore (preferred) or Ed25519 fallback
//! 2. **Capability sandbox** — actual safety floor; plugin can do nothing
//!    it doesn't import via the linker (Task 18 enforces denial)
//! 3. **Identity-pinned updates** — first install records publisher;
//!    mismatched updates refused, prompts for explicit retrust
//! 4. **First-install confirmation** — capability disclosure + override
//!    claim prominence
//!
//! ## Known limitations (Phase 1 MVP)
//!
//! - **No WASI 0.2 surface is linked.** The host wires only the six capability
//!   interfaces above; `wasi:cli/*`, `wasi:io/*`, `wasi:filesystem/*`, etc. are
//!   intentionally absent. Plugins built with `cargo-component` against a
//!   `std`-using Rust crate will silently pull WASI 0.2 imports and trap at
//!   instantiation with `component imports instance ..., but a matching
//!   implementation was not found in the linker`. Authors must build with
//!   `#![no_std]` (or use `wit-bindgen` directly without cargo-component's
//!   default WASI scaffolding) for Phase 1. Wiring a sandboxed WASI surface
//!   is tracked separately and out of scope for the MVP.
//! - **Sigstore happy-path verification** is `#[ignore]`'d in
//!   `tests/signature_sigstore.rs`: cosign 2.4 emits Bundle v0.3 protobuf
//!   format and sigstore-rs 0.13 only parses v0.1 / v0.2. Re-enable once
//!   sigstore-rs adds v0.3 parsing upstream. Sad-path tests cover the
//!   verifier's error mapping.
//!
//! ## See also
//!
//! - Design spec: `docs/superpowers/specs/2026-04-28-plugin-system-mvp-design.md` (local)
//! - Implementation plan: `docs/superpowers/plans/2026-04-28-plugin-system-mvp.md` (local)
//! - Tracking issue: <https://github.com/crippledgeek/rdlp/issues/213>
//!
//! [issue-213]: https://github.com/crippledgeek/rdlp/issues/213

#![warn(missing_docs)]

pub mod adapter;
pub mod dispatch;
pub mod engine;
pub mod error;
pub mod host;
pub mod instance;
pub mod loader;
pub mod manifest;
pub mod priority;
pub mod prompt;
pub mod signature;
pub mod trust_store;

pub use error::PluginError;

/// Generated Rust bindings from the `extractor-plugin` WIT world.
///
/// This module is regenerated at compile time by `wasmtime::component::bindgen!`.
/// It exposes:
/// - `bindings::ExtractorPlugin` — the generated host-side instance type
/// - `bindings::types::*` — record/variant types from `wit/types.wit`
/// - `bindings::host_*::Host` traits — one per imported interface, implemented
///   by the host on `PluginStoreData` (Task 11+).
///
/// Async support is enabled (matching the engine's `async_support(true)`).
#[allow(clippy::all, missing_docs)]
pub mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "extractor-plugin",
        async: true,
    });
}
