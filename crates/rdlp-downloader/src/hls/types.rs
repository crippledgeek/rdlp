//! HLS type definitions for segment and playlist parsing.

/// EXT-X-MAP initialization segment info (fMP4 streams)
///
/// Per the HLS spec, `EXT-X-MAP` applies to every segment after it until
/// the next `EXT-X-MAP` tag. A playlist may therefore contain multiple
/// init segments (e.g. codec change mid-stream, ad insertion).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InitSegmentInfo {
    /// Fully-resolved URL of the initialization segment
    pub url: String,
    /// Optional byte range `(length, offset)` when the init data is
    /// packed inside a larger resource (e.g. the same URI as the segments)
    pub byte_range: Option<(u64, Option<u64>)>,
}

/// Information about an HLS segment including its duration
#[derive(Clone, Debug)]
pub struct SegmentInfo {
    /// Segment URL
    pub url: String,
    /// Segment duration in seconds (from EXTINF)
    pub duration: f64,
    /// The EXT-X-MAP init segment that applies to this segment (if any)
    pub init_segment: Option<InitSegmentInfo>,
}

/// Result of parsing an HLS playlist
#[derive(Clone, Debug)]
pub struct PlaylistParseResult {
    /// Media segments (each carrying its own init segment reference)
    pub segments: Vec<SegmentInfo>,
}
