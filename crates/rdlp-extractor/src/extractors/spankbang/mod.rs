//! SpankBang extractor.
//!
//! Single-request extraction: fetches the video page with the `country=US`
//! cookie, parses the inline `stream_data = {...}` Python-dict for formats
//! and metadata. Falls back to the formats API (POST `/api/videos/stream`)
//! when the inline dict is absent (rare on current pages).
//!
//! Cloudflare-fronted; the wreq+BoringSSL browser-emulation HTTP client
//! shipped by `rdlp-http` is required (default in the production stack).
//!
//! Reference: yt-dlp `yt_dlp/extractor/spankbang.py` — uses `impersonate=True`
//! for the equivalent calls.

mod patterns;
mod formats;
mod metadata;

/// Public type to be implemented in Task 5. Kept here so the registry can
/// reference it once Task 5 lands.
#[derive(Default)]
pub struct SpankBangExtractor;

impl SpankBangExtractor {
    /// Construct a new extractor instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}
