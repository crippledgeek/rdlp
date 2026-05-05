//! Errors raised by the DASH MPD expander.
//!
//! Internal to the extractor crate. Convert to [`anyhow::Error`] with
//! `.context("…")` at the extractor boundary.

use thiserror::Error;

/// Failure modes during MPD → `Vec<Format>` expansion.
#[derive(Debug, Error)]
pub enum DashExpandError {
    /// The XML body could not be parsed by `dash-mpd`.
    #[error("Failed to parse MPD: {0}")]
    Parse(String),

    /// The MPD declares `@type="dynamic"` (live). Per design we refuse —
    /// pre-resolved fragments cannot represent a live, refreshing manifest.
    #[error("Dynamic/live MPD not supported")]
    DynamicMpd,

    /// All Representations were filtered out (DRM, missing segment info, etc.).
    #[error("MPD has no usable representations after filtering")]
    NoUsableReps,

    /// A resolved fragment URL or BaseURL chain pointed to a private/internal
    /// host (SSRF defence-in-depth). The MPD body or its `<BaseURL>` chain
    /// resolved against the manifest URL produces a target rejected by
    /// `rdlp_security::validate_url_security` (file://, javascript:, RFC 1918
    /// hosts, link-local 169.254.0.0/16 metadata range, etc.).
    #[error("URL rejected by SSRF gate: {0}")]
    UrlRejected(String),
}
