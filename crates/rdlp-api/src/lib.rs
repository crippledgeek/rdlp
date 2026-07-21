// Lint-tightening for LIBRARY code only. `pedantic` / `nursery` are
// stylistic; `indexing_slicing` is enforced here because production code must
// not panic on out-of-bounds. Integration tests under `tests/` deliberately
// use `vec[0]` after a length assertion as the assertion form — see
// `Cargo.toml` `[lints.clippy]` for the rationale.
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]
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
/// Pure-data option registry (config↔GUI axis).
pub mod options;
/// Download request types.
pub mod request;
/// Download result types.
pub mod result;

/// Serializable event DTOs for UI bridges (Tauri, Leptos SSE).
#[cfg(feature = "serde")]
pub mod dto;

/// Cancellation disposition: keep-vs-discard intent for partial files.
pub(crate) mod cancel;
/// Conditional merge of request overrides into Config.
pub(crate) mod merge;
/// Internal orchestrator (moved from rdlp-cli).
pub(crate) mod orchestrator;
/// Plugin system bootstrap — fail-soft loader called during client construction.
pub(crate) mod plugin_bootstrap;

// Convenience re-exports
pub use client::RdlpClient;
pub use errors::RdlpApiError;
pub use events::Event;
pub use handle::{DownloadHandle, DownloadId, InterruptHandle};
pub use orchestrator::InteractiveCallback;
pub use rdlp_core::{DownloadProgress, config_io};
pub use rdlp_postprocess::TempRegistry;
pub use rdlp_types::match_filter::MatchFilter;
pub use rdlp_types::{
    AudioFormat, BrowserEmulation, BrowserType, Codec, Config, ContainerFormat, FixupPolicy,
    Format, InfoDict, PostProcess, RecodeAudioMode, SearchFilter, SearchFilterDescriptor,
    SearchFilterValue, SearchPageResponse, SearchQuery, SearchResultPreview, SearchSiteInfo,
    SubtitleFormat,
};
pub use request::DownloadRequest;
pub use result::DownloadResult;
