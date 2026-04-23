//! Shared base for WGCZ Holding sites (XVideos, XNXX).
//!
//! Both sites embed format URLs via inline `<script>` calls
//! (`html5player.setVideoHLS(...)`, `setVideoUrlLow(...)`, `setVideoUrlHigh(...)`)
//! and expose metadata via JSON-LD `VideoObject`. This base provides parsers
//! for that shared shape.

pub mod patterns;

/// Marker struct carrying shared parse helpers for WGCZ sites.
pub struct WgczNetworkBase;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wgcz_base_is_constructible() {
        let _b = WgczNetworkBase;
    }
}
