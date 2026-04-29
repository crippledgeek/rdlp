//! `Codec` — typed video / audio codec identifier.
//!
//! Replaces the prior `Option<String>` shape on `Format::vcodec` /
//! `Format::acodec` which used the magic string `"none"` to mean "no
//! codec present" — `Some("none".to_string())` and `None` were
//! semantically identical and 25+ call sites had to remember to filter
//! the sentinel before comparing or displaying.
//!
//! ## Wire format
//!
//! Serialises as `null` for [`Codec::Absent`] and as a plain JSON string
//! (e.g. `"h264"`) for [`Codec::Present`]. Deserialisation accepts
//! `null`, the empty string, and the legacy sentinel `"none"` (case-
//! insensitive) — all collapse to [`Codec::Absent`]. Any other string
//! is preserved as [`Codec::Present`] without further normalisation.
//!
//! This means the on-the-wire / TypeScript surface is unchanged: the
//! frontend continues to see `null` or `"h264"` exactly as before.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Strongly-typed codec identifier for `Format::vcodec` / `Format::acodec`.
///
/// Construct via [`Codec::new`] / [`Codec::from_optional_str`] or directly
/// via the variants. The `"none"` sentinel is normalised to
/// [`Codec::Absent`] at every entry point.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Codec {
    /// No codec stream present (e.g. video-only file's `acodec`,
    /// audio-only file's `vcodec`, or unknown / not yet probed).
    #[default]
    Absent,
    /// A codec name as reported by the source (`"h264"`, `"vp9"`,
    /// `"opus"`, …). Stored verbatim — no canonicalisation, no closed
    /// set: forward-compatibility for codecs the host doesn't know yet.
    Present(String),
}

impl Codec {
    /// Build a `Codec` from any string, mapping the empty string and the
    /// legacy `"none"` sentinel (case-insensitive) to [`Codec::Absent`].
    #[must_use]
    pub fn new(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        if s.is_empty() || s.eq_ignore_ascii_case("none") {
            Codec::Absent
        } else {
            Codec::Present(s.to_owned())
        }
    }

    /// Convenience — same as [`Codec::new`] but accepts an `Option`.
    #[must_use]
    pub fn from_optional_str<S: AsRef<str>>(s: Option<S>) -> Self {
        s.map_or(Codec::Absent, Codec::new)
    }

    /// `true` when no codec is present.
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Codec::Absent)
    }

    /// `true` when a codec name is present.
    #[must_use]
    pub const fn is_present(&self) -> bool {
        matches!(self, Codec::Present(_))
    }

    /// Borrow the codec name when present.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Codec::Absent => None,
            Codec::Present(s) => Some(s.as_str()),
        }
    }

    /// Convert to the legacy `Option<String>` shape — for callers that
    /// haven't migrated yet. Prefer [`Codec::as_str`] for read-only access.
    #[must_use]
    pub fn into_option_string(self) -> Option<String> {
        match self {
            Codec::Absent => None,
            Codec::Present(s) => Some(s),
        }
    }
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Codec::Absent => f.write_str("none"),
            Codec::Present(s) => f.write_str(s),
        }
    }
}

// ── Conversions ──────────────────────────────────────────────────────────

impl From<&str> for Codec {
    fn from(s: &str) -> Self {
        Codec::new(s)
    }
}

impl From<String> for Codec {
    fn from(s: String) -> Self {
        Codec::new(s)
    }
}

impl From<Option<String>> for Codec {
    fn from(s: Option<String>) -> Self {
        Codec::from_optional_str(s)
    }
}

impl From<Option<&str>> for Codec {
    fn from(s: Option<&str>) -> Self {
        Codec::from_optional_str(s)
    }
}

impl From<Codec> for Option<String> {
    fn from(c: Codec) -> Self {
        c.into_option_string()
    }
}

// ── Serde — compatible with the prior `Option<String>` wire format ──────

impl Serialize for Codec {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Codec::Absent => s.serialize_none(),
            Codec::Present(name) => s.serialize_str(name),
        }
    }
}

impl<'de> Deserialize<'de> for Codec {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let opt: Option<String> = Option::deserialize(d)?;
        Ok(Codec::from_optional_str(opt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_normalises_sentinels() {
        assert_eq!(Codec::new(""), Codec::Absent);
        assert_eq!(Codec::new("none"), Codec::Absent);
        assert_eq!(Codec::new("None"), Codec::Absent);
        assert_eq!(Codec::new("NONE"), Codec::Absent);
        assert_eq!(Codec::new("h264"), Codec::Present("h264".into()));
        assert_eq!(Codec::new("vp9.0"), Codec::Present("vp9.0".into()));
    }

    #[test]
    fn predicates_invert() {
        assert!(Codec::Absent.is_absent());
        assert!(!Codec::Absent.is_present());
        let c = Codec::Present("h264".into());
        assert!(!c.is_absent());
        assert!(c.is_present());
    }

    #[test]
    fn from_optional_string_handles_sentinel() {
        assert_eq!(Codec::from_optional_str(None::<String>), Codec::Absent);
        assert_eq!(
            Codec::from_optional_str(Some("none".to_owned())),
            Codec::Absent
        );
        assert_eq!(
            Codec::from_optional_str(Some("aac".to_owned())),
            Codec::Present("aac".into())
        );
    }

    #[test]
    fn serde_round_trip_matches_legacy_wire_format() {
        // Absent serialises as JSON null
        let absent_json = serde_json::to_string(&Codec::Absent).unwrap();
        assert_eq!(absent_json, "null");

        // Present serialises as a plain string
        let present_json = serde_json::to_string(&Codec::Present("h264".into())).unwrap();
        assert_eq!(present_json, "\"h264\"");

        // Round-trip
        for c in [
            Codec::Absent,
            Codec::Present("h264".into()),
            Codec::Present("opus".into()),
        ] {
            let s = serde_json::to_string(&c).unwrap();
            assert_eq!(serde_json::from_str::<Codec>(&s).unwrap(), c);
        }

        // Backward-compat: legacy "none" / "" / null all deserialise to Absent
        for legacy in ["null", "\"\"", "\"none\"", "\"NONE\""] {
            assert_eq!(
                serde_json::from_str::<Codec>(legacy).unwrap(),
                Codec::Absent
            );
        }
    }

    #[test]
    fn display_renders_sentinel_for_absent() {
        // Display falls back to "none" so existing log lines that
        // format `format.vcodec` continue to read identically. Use
        // `is_absent` / `as_str` for logic.
        assert_eq!(format!("{}", Codec::Absent), "none");
        assert_eq!(format!("{}", Codec::Present("h264".into())), "h264");
    }

    #[test]
    fn from_str_and_string_normalise() {
        let from_str: Codec = "none".into();
        assert_eq!(from_str, Codec::Absent);
        let from_string: Codec = String::from("h264").into();
        assert_eq!(from_string, Codec::Present("h264".into()));
    }
}
