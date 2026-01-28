//! HLS codec parsing utilities
//!
//! Parses the HLS `CODECS` attribute (RFC 6381) from M3U8 playlists
//! into human-readable video and audio codec names.

/// Parse an HLS `CODECS` attribute string into video and audio codec names.
///
/// The CODECS attribute in HLS master playlists uses RFC 6381 format,
/// e.g., `"avc1.64001f,mp4a.40.2"`. This function splits the string
/// and maps each codec identifier to a human-readable name.
///
/// # Arguments
/// * `codecs` - The CODECS attribute value (e.g., `"avc1.64001f,mp4a.40.2"`)
///
/// # Returns
/// A tuple of `(video_codec, audio_codec)` where each is `None` if not found.
///
/// # Examples
/// ```
/// use rdlp_core::codecs::parse_hls_codecs;
///
/// let (video, audio) = parse_hls_codecs("avc1.64001f,mp4a.40.2");
/// assert_eq!(video, Some("h264"));
/// assert_eq!(audio, Some("aac"));
///
/// let (video, audio) = parse_hls_codecs("hvc1.1.6.L93.B0");
/// assert_eq!(video, Some("hevc"));
/// assert_eq!(audio, None);
/// ```
pub fn parse_hls_codecs(codecs: &str) -> (Option<&'static str>, Option<&'static str>) {
    let mut video = None;
    let mut audio = None;

    for codec in codecs.split(',').map(str::trim) {
        if codec.is_empty() {
            continue;
        }

        // Extract the codec family prefix (before the first dot)
        let prefix = codec.split('.').next().unwrap_or(codec);

        match prefix {
            // Video codecs
            "avc1" | "avc3" => video = Some("h264"),
            "hvc1" | "hev1" => video = Some("hevc"),
            "vp09" => video = Some("vp9"),
            "vp8" | "vp08" => video = Some("vp8"),
            "av01" => video = Some("av1"),
            // Audio codecs
            "mp4a" => audio = Some("aac"),
            "ac-3" => audio = Some("ac3"),
            "ec-3" => audio = Some("eac3"),
            "opus" => audio = Some("opus"),
            "flac" => audio = Some("flac"),
            "vorbis" => audio = Some("vorbis"),
            _ => {} // Unknown codec prefix, skip
        }
    }

    (video, audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h264_aac() {
        let (v, a) = parse_hls_codecs("avc1.64001f,mp4a.40.2");
        assert_eq!(v, Some("h264"));
        assert_eq!(a, Some("aac"));
    }

    #[test]
    fn test_hevc_aac() {
        let (v, a) = parse_hls_codecs("hvc1.1.6.L93.B0,mp4a.40.2");
        assert_eq!(v, Some("hevc"));
        assert_eq!(a, Some("aac"));
    }

    #[test]
    fn test_hev1_variant() {
        let (v, a) = parse_hls_codecs("hev1.1.6.L120.90,mp4a.40.5");
        assert_eq!(v, Some("hevc"));
        assert_eq!(a, Some("aac"));
    }

    #[test]
    fn test_vp9() {
        let (v, a) = parse_hls_codecs("vp09.00.10.08");
        assert_eq!(v, Some("vp9"));
        assert_eq!(a, None);
    }

    #[test]
    fn test_av1_opus() {
        let (v, a) = parse_hls_codecs("av01.0.08M.08,opus");
        assert_eq!(v, Some("av1"));
        assert_eq!(a, Some("opus"));
    }

    #[test]
    fn test_audio_only_aac() {
        let (v, a) = parse_hls_codecs("mp4a.40.2");
        assert_eq!(v, None);
        assert_eq!(a, Some("aac"));
    }

    #[test]
    fn test_audio_only_ac3() {
        let (v, a) = parse_hls_codecs("ac-3");
        assert_eq!(v, None);
        assert_eq!(a, Some("ac3"));
    }

    #[test]
    fn test_audio_only_eac3() {
        let (v, a) = parse_hls_codecs("ec-3");
        assert_eq!(v, None);
        assert_eq!(a, Some("eac3"));
    }

    #[test]
    fn test_empty_string() {
        let (v, a) = parse_hls_codecs("");
        assert_eq!(v, None);
        assert_eq!(a, None);
    }

    #[test]
    fn test_unknown_codec() {
        let (v, a) = parse_hls_codecs("unknown.codec");
        assert_eq!(v, None);
        assert_eq!(a, None);
    }

    #[test]
    fn test_whitespace_handling() {
        let (v, a) = parse_hls_codecs(" avc1.64001f , mp4a.40.2 ");
        assert_eq!(v, Some("h264"));
        assert_eq!(a, Some("aac"));
    }

    #[test]
    fn test_avc3_variant() {
        let (v, _) = parse_hls_codecs("avc3.64001f");
        assert_eq!(v, Some("h264"));
    }

    #[test]
    fn test_vp8() {
        let (v, a) = parse_hls_codecs("vp8,vorbis");
        assert_eq!(v, Some("vp8"));
        assert_eq!(a, Some("vorbis"));
    }

    #[test]
    fn test_flac() {
        let (v, a) = parse_hls_codecs("flac");
        assert_eq!(v, None);
        assert_eq!(a, Some("flac"));
    }
}
