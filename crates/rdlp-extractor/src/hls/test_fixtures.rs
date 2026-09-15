//! HLS playlist fixtures shared across `rdlp-extractor`'s own unit tests and,
//! via the `loopback-test-exemption` feature, `rdlp-plugin`'s host-import
//! tests (`expand-hls` / `probe-format-sizes` wrap the same in-tree helpers
//! these fixtures already exercise here).
//!
//! Moved out of `test_support` because `test_support` is
//! `#[cfg(test)] pub(crate)` — invisible outside this crate even in a
//! feature-enabled build, since `cfg(test)` never applies to a dependent
//! crate's compilation of this one. This module instead gates on
//! `any(test, feature = "loopback-test-exemption")` and is `pub`, so a
//! sibling crate that turns the feature on can actually reach it.

/// Master playlist with two video variants. Used by probe-order regression tests.
pub const MASTER_TWO_VARIANTS: &str = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=1280x720\n\
v720.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=640000,RESOLUTION=640x360\n\
v360.m3u8\n";

/// Variant media playlist body — two segments, ENDLIST'd.
pub const VARIANT_MEDIA: &str = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXTINF:6.0,\n\
seg-1.ts\n\
#EXTINF:6.0,\n\
seg-2.ts\n\
#EXT-X-ENDLIST\n";
