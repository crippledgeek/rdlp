//! HLS playlist fixtures shared across `rdlp-extractor`'s own unit tests and,
//! via the `loopback-test-exemption` feature, `rdlp-plugin`'s host-import
//! tests (`expand-hls` / `probe-format-sizes` wrap the same in-tree helpers
//! these fixtures already exercise here). Moved out of `test_support` so a
//! sibling crate can depend on the fixtures without depending on the mockito
//! loopback bypass more broadly than this one pair of playlists.

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
