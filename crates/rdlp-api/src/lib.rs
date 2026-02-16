#![warn(missing_docs)]
//! Frontend-agnostic API for the rdlp download engine.
//!
//! This crate provides a stable event model, error types, and download
//! handle for managing concurrent downloads from any frontend
//! (CLI, Tauri, Leptos).

/// Optional event fan-out for multi-subscriber scenarios.
pub mod bus;
/// Stable error types for the public API.
pub mod errors;
/// Download lifecycle events.
pub mod events;
/// Download handle and ID types.
pub mod handle;
/// Download request types.
pub mod request;
/// Download result types.
pub mod result;

/// Serializable event DTOs for UI bridges (Tauri, Leptos SSE).
#[cfg(feature = "serde")]
pub mod dto;
