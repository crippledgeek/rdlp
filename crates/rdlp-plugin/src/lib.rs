//! # rdlp-plugin
//!
//! Plugin system for rdlp. Loads polyglot WASM components implementing the
//! `rdlp:plugin/extractor` WIT contract and registers them as InfoExtractors.

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
