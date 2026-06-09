//! libvpx `-deadline` quality/speed tradeoff setting (VP8/VP9).

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// libvpx `-deadline` value (VP8/VP9). Named-int alias in `FFmpeg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VpxDeadline {
    /// Best quality; slowest encode. `FFmpeg` option value: `best`.
    Best,
    /// Balanced quality/speed tradeoff. `FFmpeg` option value: `good`.
    Good,
    /// Fastest encode; lowest quality. `FFmpeg` option value: `realtime`.
    Realtime,
}

impl VpxDeadline {
    /// `FFmpeg` `AVOption` value (resolved by `av_opt_set` as a named int constant).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::Good => "good",
            Self::Realtime => "realtime",
        }
    }
}

impl std::fmt::Display for VpxDeadline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for VpxDeadline {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "best" => Ok(Self::Best),
            "good" => Ok(Self::Good),
            "realtime" => Ok(Self::Realtime),
            other => Err(format!(
                "invalid deadline '{other}' (expected best|good|realtime)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_case_insensitively() {
        assert_eq!("Good".parse::<VpxDeadline>().unwrap(), VpxDeadline::Good);
        assert_eq!(
            "realtime".parse::<VpxDeadline>().unwrap(),
            VpxDeadline::Realtime
        );
    }

    #[test]
    fn rejects_unknown() {
        assert!("medium".parse::<VpxDeadline>().is_err());
    }

    #[test]
    fn serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&VpxDeadline::Best).unwrap(),
            "\"best\""
        );
        assert_eq!(
            serde_json::from_str::<VpxDeadline>("\"realtime\"").unwrap(),
            VpxDeadline::Realtime
        );
    }

    #[test]
    fn as_str_roundtrips() {
        for d in [VpxDeadline::Best, VpxDeadline::Good, VpxDeadline::Realtime] {
            assert_eq!(d.as_str().parse::<VpxDeadline>().unwrap(), d);
        }
    }

    #[test]
    fn displays_as_str() {
        assert_eq!(format!("{}", VpxDeadline::Good), "good");
    }
}
