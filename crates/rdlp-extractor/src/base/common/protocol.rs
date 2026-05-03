//! URL → `DownloadProtocol` classifier based on path-segment extension.
//!
//! Mirrors the heuristic used by FFmpeg (`libavformat/hls.c`), VLC
//! (`modules/demux/playlist/m3u.c`), mpv (`demux/demux_lavf.c`), and yt-dlp
//! (`utils/_utils.py::determine_ext`): query and fragment are ignored.
//!
//! Callers that already have out-of-band knowledge of the protocol (e.g. a
//! JS field literally named `setVideoHLS(...)`) MUST set the protocol
//! directly; this helper is the fallback for pure URL strings.

use rdlp_types::DownloadProtocol;
use url::Url;

/// Classify a URL into a [`DownloadProtocol`] by inspecting the last path
/// segment's extension. Query and fragment are ignored.
///
/// Returns:
/// - [`DownloadProtocol::M3u8`] for `.m3u8` / `.m3u` (case-insensitive)
/// - [`DownloadProtocol::Https`] / [`DownloadProtocol::Http`] for the matching scheme
/// - [`DownloadProtocol::Other`] for any other scheme (e.g. `data:`, `javascript:`)
#[must_use]
pub(crate) fn protocol_for_url(url: &Url) -> DownloadProtocol {
    let ext = url
        .path()
        .rsplit('/')
        .next()
        .and_then(|seg| seg.rsplit_once('.').map(|(_, ext)| ext))
        .unwrap_or("");

    if ext.eq_ignore_ascii_case("m3u8") || ext.eq_ignore_ascii_case("m3u") {
        return DownloadProtocol::M3u8;
    }

    match url.scheme() {
        "https" => DownloadProtocol::Https,
        "http" => DownloadProtocol::Http,
        s => DownloadProtocol::Other(s.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn m3u8_path_extension_classifies_as_hls() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/master.m3u8")),
            DownloadProtocol::M3u8
        ));
    }

    #[test]
    fn m3u8_with_query_and_fragment_classifies_as_hls() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/path/master.m3u8?token=abc#frag")),
            DownloadProtocol::M3u8
        ));
    }

    #[test]
    fn m3u8_uppercase_classifies_as_hls() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/PLAYLIST.M3U8")),
            DownloadProtocol::M3u8
        ));
    }

    #[test]
    fn m3u_path_extension_classifies_as_hls() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/playlist.m3u")),
            DownloadProtocol::M3u8
        ));
    }

    #[test]
    fn mp4_with_m3u8_in_query_classifies_as_https() {
        // Issue #268 reference case.
        assert!(matches!(
            protocol_for_url(&parse("https://host/clip.mp4?ref=foo.m3u8")),
            DownloadProtocol::Https
        ));
    }

    #[test]
    fn mp4_with_m3u8_in_fragment_classifies_as_https() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/clip.mp4#foo.m3u8")),
            DownloadProtocol::Https
        ));
    }

    #[test]
    fn webm_classifies_as_https() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/video.webm")),
            DownloadProtocol::Https
        ));
    }

    #[test]
    fn mkv_classifies_as_https() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/video.mkv")),
            DownloadProtocol::Https
        ));
    }

    #[test]
    fn http_scheme_classifies_as_http() {
        assert!(matches!(
            protocol_for_url(&parse("http://host/video.mp4")),
            DownloadProtocol::Http
        ));
    }

    #[test]
    fn no_extension_classifies_as_https() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/no-extension/abc123")),
            DownloadProtocol::Https
        ));
    }

    #[test]
    fn trailing_slash_classifies_as_https() {
        assert!(matches!(
            protocol_for_url(&parse("https://host/path/")),
            DownloadProtocol::Https
        ));
    }

    #[test]
    fn data_scheme_classifies_as_other() {
        let p = protocol_for_url(&parse("data:text/plain,foo"));
        match p {
            DownloadProtocol::Other(s) => assert_eq!(s, "data"),
            _ => panic!("expected Other(\"data\"), got {p:?}"),
        }
    }
}
