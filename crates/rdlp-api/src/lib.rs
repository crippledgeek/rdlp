#![warn(missing_docs)]
//! Frontend-agnostic API for the rdlp download engine.
//!
//! This crate provides a stable event model, error types, and download
//! handle for managing concurrent downloads from any frontend
//! (CLI, Tauri, Leptos).

/// Download handle and ID types.
pub mod handle;
/// Download result types.
pub mod result;
