//! Download protocol types

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Download protocol for a format stream.
///
/// Determines which downloader handles the stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadProtocol {
    /// Plain HTTP
    Http,
    /// HTTPS
    Https,
    /// HLS (M3U8) playlist
    M3u8,
    /// Native HLS implementation
    M3u8Native,
    /// DASH segments over HTTP
    HttpDashSegments,
    /// Other/unknown protocol
    Other(String),
}

impl DownloadProtocol {
    /// Check if this is an HLS protocol.
    #[must_use]
    pub fn is_hls(&self) -> bool {
        matches!(self, Self::M3u8 | Self::M3u8Native)
    }

    /// Check if this is a DASH protocol.
    #[must_use]
    pub fn is_dash(&self) -> bool {
        matches!(self, Self::HttpDashSegments)
    }

    /// Get the protocol as a string slice.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::M3u8 => "m3u8",
            Self::M3u8Native => "m3u8_native",
            Self::HttpDashSegments => "http_dash_segments",
            Self::Other(s) => s,
        }
    }
}

impl fmt::Display for DownloadProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DownloadProtocol {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(if s.eq_ignore_ascii_case("http") {
            Self::Http
        } else if s.eq_ignore_ascii_case("https") {
            Self::Https
        } else if s.eq_ignore_ascii_case("m3u8") {
            Self::M3u8
        } else if s.eq_ignore_ascii_case("m3u8_native") {
            Self::M3u8Native
        } else if s.eq_ignore_ascii_case("http_dash_segments") {
            Self::HttpDashSegments
        } else {
            Self::Other(s.to_string())
        })
    }
}

impl From<String> for DownloadProtocol {
    fn from(s: String) -> Self {
        if s.eq_ignore_ascii_case("http") {
            Self::Http
        } else if s.eq_ignore_ascii_case("https") {
            Self::Https
        } else if s.eq_ignore_ascii_case("m3u8") {
            Self::M3u8
        } else if s.eq_ignore_ascii_case("m3u8_native") {
            Self::M3u8Native
        } else if s.eq_ignore_ascii_case("http_dash_segments") {
            Self::HttpDashSegments
        } else {
            Self::Other(s)
        }
    }
}

impl From<&str> for DownloadProtocol {
    fn from(s: &str) -> Self {
        s.parse().expect("DownloadProtocol::from_str is infallible")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hls() {
        assert!(DownloadProtocol::M3u8.is_hls());
        assert!(DownloadProtocol::M3u8Native.is_hls());
        assert!(!DownloadProtocol::Https.is_hls());
        assert!(!DownloadProtocol::HttpDashSegments.is_hls());
    }

    #[test]
    fn test_is_dash() {
        assert!(DownloadProtocol::HttpDashSegments.is_dash());
        assert!(!DownloadProtocol::Https.is_dash());
    }

    #[test]
    fn test_parse_roundtrip() {
        for proto in [
            DownloadProtocol::Http,
            DownloadProtocol::Https,
            DownloadProtocol::M3u8,
            DownloadProtocol::M3u8Native,
            DownloadProtocol::HttpDashSegments,
        ] {
            let s = proto.to_string();
            let parsed: DownloadProtocol = s.parse().unwrap();
            assert_eq!(proto, parsed);
        }
    }

    #[test]
    fn test_unknown_protocol() {
        let proto: DownloadProtocol = "rtmp".parse().unwrap();
        assert_eq!(proto, DownloadProtocol::Other("rtmp".to_string()));
        assert_eq!(proto.as_str(), "rtmp");
    }

    #[test]
    fn test_serde_roundtrip() {
        let proto = DownloadProtocol::M3u8Native;
        let json = serde_json::to_string(&proto).unwrap();
        assert_eq!(json, "\"m3u8_native\"");
        let parsed: DownloadProtocol = serde_json::from_str(&json).unwrap();
        assert_eq!(proto, parsed);
    }
}
