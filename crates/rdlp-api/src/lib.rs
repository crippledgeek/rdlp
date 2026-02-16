#![warn(missing_docs)]
//! Frontend-agnostic API for the rdlp download engine.
//!
//! [`RdlpClient`] is the primary entry point for all frontends
//! (CLI, Tauri, Leptos). It exposes a stable event model, error types,
//! and download handle for managing concurrent downloads.
//!
//! # Features
//!
//! - `serde` — Enables [`dto::EventDto`] for JSON serialization of events.

/// Optional event fan-out for multi-subscriber scenarios.
pub mod bus;
/// Primary API client and builder.
pub mod client;
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

/// Internal orchestrator (moved from rdlp-cli).
pub(crate) mod orchestrator;

// Convenience re-exports
pub use client::RdlpClient;
pub use errors::RdlpApiError;
pub use events::Event;
pub use handle::{DownloadHandle, DownloadId};
pub use orchestrator::InteractiveCallback;
pub use request::DownloadRequest;
pub use result::DownloadResult;
