//! XNXX extractor
//!
//! XNXX is a WGCZ Holding site (same inline-JS format as XVideos).
//! Inline `html5player.setXxx` calls provide HLS and MP4 format URLs.
//!
//! Supports:
//! - Video pages: `https://www.xnxx.com/video-14cco143/slug`
//! - Video pages (no hyphen): `https://www.xnxx.com/video14cco143/slug`
//! - xnxx3.com variant: `https://www.xnxx3.com/video-14cco143/slug`
//! - Embed pages: `https://www.xnxx.com/embedframe/14cco143`
//!
//! ## Module structure
//!
//! - `patterns` - URL regex patterns
//! - `search`   - SearchExtractor implementation

pub mod patterns;

/// XNXX site extractor.
#[derive(Default)]
pub struct XNXXExtractor;

impl XNXXExtractor {
    /// Create a new XNXX extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}
