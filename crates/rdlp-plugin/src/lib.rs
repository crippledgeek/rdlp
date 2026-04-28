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
