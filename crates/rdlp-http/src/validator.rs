//! Strong validators for range-safe resume (RFC 9110 §8.8, §13.1.5).
//!
//! Values are kept **verbatim** as received: the server's `If-Range` check is
//! an exact match on the field value (§13.1.5) and an entity tag is opaque
//! octets (§8.8.3.1). Parsing here only classifies — strong vs weak, IMF vs
//! obsolete date — and never regenerates what is sent back.

use std::ops::RangeInclusive;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use wreq::header::{HeaderMap, HeaderValue};

/// Minimum gap between `Date` and `Last-Modified` for the date to count as a
/// strong validator when a client uses it in `If-Range` (§8.8.2.2: "at least
/// one second after").
const LAST_MODIFIED_STRENGTH_GAP: chrono::TimeDelta = chrono::TimeDelta::seconds(1);

/// §5.6.7 `IMF-fixdate`; the only format a sender may generate.
const IMF_FIXDATE: &str = "%a, %d %b %Y %H:%M:%S GMT";
/// §5.6.7 obsolete RFC 850 format (`Sunday, 06-Nov-94 08:49:37 GMT`).
const RFC850_DATE: &str = "%A, %d-%b-%y %H:%M:%S GMT";
/// §5.6.7 obsolete asctime format (`Sun Nov  6 08:49:37 1994`); `%e` is the
/// space-padded day.
const ASCTIME_DATE: &str = "%a %b %e %H:%M:%S %Y";

/// §8.8.3 `etagc = %x21 / %x23-7E / obs-text` — the exclamation mark, the
/// lone byte excluded on either side of it (`)`, `%x22 DQUOTE`, is handled by
/// the surrounding-quote check, not this constant).
const ETAGC_BANG: u8 = 0x21;
/// §8.8.3 `etagc` printable range: everything from `#` to `~`, which skips
/// `DQUOTE` (0x22, the delimiter) and `DEL` (0x7F).
const ETAGC_RANGE: RangeInclusive<u8> = 0x23..=0x7E;
/// §8.8.3 `obs-text = %x80-FF`, the start of the obsolete high-bit range.
const OBS_TEXT_START: u8 = 0x80;

/// An entity tag known to be strong: `DQUOTE *etagc DQUOTE` with no `W/`.
///
/// Carries the parsed `HeaderValue` alongside the raw `String` so
/// `StrongValidator::if_range_value` never has to re-validate (and can never
/// fail to) turn the stored bytes back into a header value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrongEntityTag(String, HeaderValue);

impl StrongEntityTag {
    /// `None` for a weak tag, an unquoted value, or a byte outside `etagc`
    /// (§8.8.3: `%x21 / %x23-7E / obs-text`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let bytes = raw.as_bytes();
        if bytes.len() < 2 {
            return None;
        }
        if *bytes.first()? != b'"' || *bytes.last()? != b'"' {
            return None;
        }
        // `len >= 2` above makes `len - 1 >= 1`, so this range never inverts.
        let inner = bytes.get(1..bytes.len() - 1)?;
        let etagc = |b: &u8| *b == ETAGC_BANG || ETAGC_RANGE.contains(b) || *b >= OBS_TEXT_START;
        if !inner.iter().all(etagc) {
            return None;
        }
        // `etagc` (and the DQUOTE delimiters, 0x22) is a subset of the byte
        // range `HeaderValue::from_bytes` accepts, so this Option is always
        // `Some` for input that reached this line — but building it here,
        // once, is what makes `if_range_value` infallible by construction
        // rather than by an unenforced claim about `parse`'s own bytes.
        let header_value = HeaderValue::from_bytes(bytes).ok()?;
        Some(Self(raw.to_owned(), header_value))
    }

    /// The raw field value, quotes included.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A `Last-Modified` value in IMF-fixdate form, kept verbatim.
///
/// Carries the parsed `HeaderValue` alongside the raw `String` for the same
/// reason as [`StrongEntityTag`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImfFixdate(String, HeaderValue);

impl ImfFixdate {
    /// `None` for the two obsolete formats: echoing them would break §5.6.7's
    /// sender rule and re-serializing would break the server's exact match.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        NaiveDateTime::parse_from_str(raw, IMF_FIXDATE).ok()?;
        // IMF-fixdate is pure visible ASCII, always inside `HeaderValue`'s
        // accepted byte range; see the note in `StrongEntityTag::parse`.
        let header_value = HeaderValue::from_bytes(raw.as_bytes()).ok()?;
        Some(Self(raw.to_owned(), header_value))
    }

    /// The raw field value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Decode a header field value as UTF-8.
///
/// `HeaderValue::to_str` rejects any byte `>= 0x80` outright, which would
/// also reject a legitimate `obs-text` entity-tag byte (§8.8.3) the moment
/// it round-trips through a header map — the field grammar allows those
/// bytes; only `to_str`'s narrower "visible ASCII" promise does not.
fn header_value_as_str(v: &HeaderValue) -> Option<&str> {
    std::str::from_utf8(v.as_bytes()).ok()
}

/// Parse any of the three §5.6.7 formats; `%S` accepts `60` (leap second).
#[must_use]
pub fn parse_http_date(raw: &str) -> Option<DateTime<Utc>> {
    [IMF_FIXDATE, RFC850_DATE, ASCTIME_DATE]
        .iter()
        .find_map(|fmt| NaiveDateTime::parse_from_str(raw.trim(), fmt).ok())
        .map(|naive| naive.and_utc())
}

/// A validator that may be sent in `If-Range` (§13.1.5) and used to combine
/// parts (§15.3.7.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum StrongValidator {
    /// A strong `ETag`, preferred whenever the response offers one.
    ETag(StrongEntityTag),
    /// A `Last-Modified` at least one second before `Date` (§8.8.2.2).
    LastModified(ImfFixdate),
}

const ETAG_PREFIX: &str = "etag:";
const LAST_MODIFIED_PREFIX: &str = "last-modified:";

impl From<StrongValidator> for String {
    fn from(v: StrongValidator) -> Self {
        match v {
            StrongValidator::ETag(t) => format!("{ETAG_PREFIX}{}", t.0),
            StrongValidator::LastModified(d) => format!("{LAST_MODIFIED_PREFIX}{}", d.0),
        }
    }
}

impl TryFrom<String> for StrongValidator {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        if let Some(raw) = s.strip_prefix(ETAG_PREFIX) {
            return StrongEntityTag::parse(raw)
                .map(Self::ETag)
                .ok_or_else(|| format!("not a strong entity-tag: {raw}"));
        }
        if let Some(raw) = s.strip_prefix(LAST_MODIFIED_PREFIX) {
            return ImfFixdate::parse(raw)
                .map(Self::LastModified)
                .ok_or_else(|| format!("not an IMF-fixdate: {raw}"));
        }
        Err(format!("unknown validator prefix: {s}"))
    }
}

/// Why a 206's headers do not agree with the validator that was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatorMismatch {
    /// The response carried no `ETag` although §15.3.7 requires one on a 206.
    Missing,
    /// The response's `ETag` is weak and so cannot match strongly (§8.8.3.2).
    Weak(String),
    /// A different validator: the server served another representation.
    Different {
        /// The validator that was sent in `If-Range`.
        expected: String,
        /// The validator the 206 response actually carried.
        got: String,
    },
}

impl std::fmt::Display for ValidatorMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => {
                f.write_str("206 response carries no ETag (RFC 9110 §15.3.7 requires it)")
            }
            Self::Weak(got) => write!(
                f,
                "206 response carries a weak ETag {got}; strong comparison fails"
            ),
            Self::Different { expected, got } => {
                write!(
                    f,
                    "validator changed: sent {expected}, response carries {got}"
                )
            }
        }
    }
}

impl StrongValidator {
    /// Select the validator a response offers, per §13.1.5's send rules.
    ///
    /// An `ETag` header that is weak or malformed yields `None` outright —
    /// no `Last-Modified` fallback — because an HTTP-date may be sent only
    /// when "the client has no entity tag", and a weak tag is still one.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        if let Some(etag) = headers.get("etag") {
            return StrongEntityTag::parse(header_value_as_str(etag)?).map(Self::ETag);
        }
        let last_modified = header_value_as_str(headers.get("last-modified")?)?;
        // §6.6.1: only the server's own Date can adjudicate strength; the
        // receipt time is not the server's clock.
        let date = parse_http_date(header_value_as_str(headers.get("date")?)?)?;
        let modified = parse_http_date(last_modified)?;
        (date - modified >= LAST_MODIFIED_STRENGTH_GAP)
            .then(|| ImfFixdate::parse(last_modified))
            .flatten()
            .map(Self::LastModified)
    }

    /// The raw bytes, for `If-Range`.
    ///
    /// Clones the `HeaderValue` `StrongEntityTag`/`ImfFixdate` built at parse
    /// time — no re-validation, and no panicking fallback, is possible here.
    #[must_use]
    pub fn if_range_value(&self) -> HeaderValue {
        match self {
            Self::ETag(t) => t.1.clone(),
            Self::LastModified(d) => d.1.clone(),
        }
    }

    /// Check a 206's headers against the validator that was sent.
    ///
    /// `ETag` is in §15.3.7's MUST-repeat list, so its absence is a
    /// mismatch; `Last-Modified` is not, and a 206 to an If-Range request
    /// SHOULD NOT repeat it, so its absence means "condition true".
    ///
    /// # Errors
    ///
    /// Returns [`ValidatorMismatch`] when the response's validator is
    /// missing, weak, or names a different representation than the one
    /// `If-Range` was sent against.
    pub fn verify_partial(&self, headers: &HeaderMap) -> Result<(), ValidatorMismatch> {
        match self {
            Self::ETag(expected) => {
                let got = headers
                    .get("etag")
                    .and_then(header_value_as_str)
                    .ok_or(ValidatorMismatch::Missing)?;
                match StrongEntityTag::parse(got) {
                    Some(tag) if tag == *expected => Ok(()),
                    Some(tag) => Err(ValidatorMismatch::Different {
                        expected: expected.as_str().to_owned(),
                        got: tag.as_str().to_owned(),
                    }),
                    None => Err(ValidatorMismatch::Weak(got.to_owned())),
                }
            }
            Self::LastModified(expected) => {
                match headers.get("last-modified").and_then(header_value_as_str) {
                    None => Ok(()),
                    Some(got) if got == expected.as_str() => Ok(()),
                    Some(got) => Err(ValidatorMismatch::Different {
                        expected: expected.as_str().to_owned(),
                        got: got.to_owned(),
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wreq::header::{HeaderMap, HeaderValue};

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    // §8.8.3 grammar: entity-tag = [ weak ] opaque-tag; weak = %s"W/" (case-sensitive)
    #[test]
    fn strong_etag_parses_verbatim() {
        assert_eq!(
            StrongEntityTag::parse("\"abc\"").unwrap().as_str(),
            "\"abc\""
        );
    }
    #[test]
    fn empty_opaque_tag_is_a_valid_strong_etag() {
        assert!(StrongEntityTag::parse("\"\"").is_some());
    }
    #[test]
    fn weak_etag_is_not_strong() {
        assert!(StrongEntityTag::parse("W/\"abc\"").is_none());
    }
    #[test]
    fn lowercase_w_slash_is_not_an_entity_tag() {
        assert!(StrongEntityTag::parse("w/\"abc\"").is_none());
    }
    #[test]
    fn unquoted_etag_is_rejected() {
        assert!(StrongEntityTag::parse("abc").is_none());
    }
    #[test]
    fn etag_with_embedded_dquote_is_rejected() {
        assert!(StrongEntityTag::parse("\"a\"b\"").is_none());
    }
    // §8.8.3 `etagc`'s `obs-text = %x80-FF` arm: a non-ASCII opaque-tag byte
    // (UTF-8 `é` is 0xC3 0xA9, both >= 0x80) is still a valid strong etag.
    #[test]
    fn obs_text_byte_in_opaque_tag_is_a_valid_strong_etag() {
        let raw = "\"caf\u{e9}\"";
        let tag = StrongEntityTag::parse(raw).unwrap();
        assert_eq!(tag.as_str(), raw);
    }
    #[test]
    fn obs_text_etag_if_range_value_round_trips_the_same_bytes() {
        let raw = "\"caf\u{e9}\"";
        let v = StrongValidator::ETag(StrongEntityTag::parse(raw).unwrap());
        assert_eq!(
            v.if_range_value(),
            HeaderValue::from_bytes(raw.as_bytes()).unwrap()
        );
    }
    #[test]
    fn obs_text_etag_verify_partial_against_matching_header_passes() {
        let raw = "\"caf\u{e9}\"";
        let v = StrongValidator::ETag(StrongEntityTag::parse(raw).unwrap());
        assert_eq!(v.verify_partial(&headers(&[("etag", raw)])), Ok(()));
    }

    // §5.6.7: a recipient MUST accept all three formats; %S allows 60 (leap second)
    #[test]
    fn parses_imf_fixdate() {
        assert!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").is_some());
    }
    #[test]
    fn parses_rfc850_date() {
        assert!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT").is_some());
    }
    #[test]
    fn parses_asctime_date() {
        assert!(parse_http_date("Sun Nov  6 08:49:37 1994").is_some());
    }
    #[test]
    fn parses_leap_second() {
        assert!(parse_http_date("Sat, 31 Dec 2016 23:59:60 GMT").is_some());
    }
    #[test]
    fn three_formats_agree_on_the_instant() {
        let a = parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").unwrap();
        let b = parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT").unwrap();
        let c = parse_http_date("Sun Nov  6 08:49:37 1994").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
    }
    #[test]
    fn garbage_date_is_rejected() {
        assert!(parse_http_date("yesterday").is_none());
    }

    // Only IMF-fixdate may be echoed back (§5.6.7 sender MUST generate IMF-fixdate)
    #[test]
    fn imf_fixdate_newtype_accepts_only_imf() {
        assert!(ImfFixdate::parse("Sun, 06 Nov 1994 08:49:37 GMT").is_some());
        assert!(ImfFixdate::parse("Sunday, 06-Nov-94 08:49:37 GMT").is_none());
        assert!(ImfFixdate::parse("Sun Nov  6 08:49:37 1994").is_none());
    }

    // from_headers selection rules
    #[test]
    fn strong_etag_wins_over_last_modified() {
        let h = headers(&[
            ("etag", "\"v1\""),
            ("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("date", "Sun, 06 Nov 1994 09:00:00 GMT"),
        ]);
        assert_eq!(
            StrongValidator::from_headers(&h),
            Some(StrongValidator::ETag(
                StrongEntityTag::parse("\"v1\"").unwrap()
            ))
        );
    }
    #[test]
    fn weak_etag_blocks_last_modified_fallback() {
        // §13.1.5: HTTP-date allowed only when "the client has no entity tag";
        // a weak tag is still an entity tag.
        let h = headers(&[
            ("etag", "W/\"v1\""),
            ("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("date", "Sun, 06 Nov 1994 09:00:00 GMT"),
        ]);
        assert_eq!(StrongValidator::from_headers(&h), None);
    }
    #[test]
    fn last_modified_without_date_is_not_a_validator() {
        let h = headers(&[("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT")]);
        assert_eq!(StrongValidator::from_headers(&h), None);
    }
    #[test]
    fn last_modified_same_second_as_date_is_weak() {
        // §8.8.2.2: Date must be at least one second after Last-Modified
        let h = headers(&[
            ("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("date", "Sun, 06 Nov 1994 08:49:37 GMT"),
        ]);
        assert_eq!(StrongValidator::from_headers(&h), None);
    }
    #[test]
    fn last_modified_one_second_before_date_is_strong() {
        let h = headers(&[
            ("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("date", "Sun, 06 Nov 1994 08:49:38 GMT"),
        ]);
        assert!(matches!(
            StrongValidator::from_headers(&h),
            Some(StrongValidator::LastModified(_))
        ));
    }
    #[test]
    fn last_modified_two_seconds_before_date_is_strong() {
        let h = headers(&[
            ("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("date", "Sun, 06 Nov 1994 08:49:39 GMT"),
        ]);
        assert!(StrongValidator::from_headers(&h).is_some());
    }
    #[test]
    fn last_modified_in_obsolete_format_is_rejected_even_with_date() {
        let h = headers(&[
            ("last-modified", "Sunday, 06-Nov-94 08:49:37 GMT"),
            ("date", "Sun, 06 Nov 1994 09:00:00 GMT"),
        ]);
        assert_eq!(StrongValidator::from_headers(&h), None);
    }
    #[test]
    fn date_in_obsolete_format_still_parses_for_the_strength_check() {
        let h = headers(&[
            ("last-modified", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("date", "Sunday, 06-Nov-94 09:00:00 GMT"),
        ]);
        assert!(StrongValidator::from_headers(&h).is_some());
    }
    #[test]
    fn no_validator_headers_gives_none() {
        assert_eq!(StrongValidator::from_headers(&HeaderMap::new()), None);
    }

    // If-Range value is the raw bytes (§13.1.5 exact match)
    #[test]
    fn if_range_value_is_verbatim() {
        let v = StrongValidator::ETag(StrongEntityTag::parse("\"x/y\"").unwrap());
        assert_eq!(v.if_range_value(), HeaderValue::from_static("\"x/y\""));
        let d = StrongValidator::LastModified(
            ImfFixdate::parse("Sun, 06 Nov 1994 08:49:37 GMT").unwrap(),
        );
        assert_eq!(
            d.if_range_value(),
            HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT")
        );
    }

    // Serde round-trip preserves raw bytes
    #[test]
    fn serde_round_trips_etag_and_date() {
        for v in [
            StrongValidator::ETag(StrongEntityTag::parse("\"abc\"").unwrap()),
            StrongValidator::LastModified(
                ImfFixdate::parse("Sun, 06 Nov 1994 08:49:37 GMT").unwrap(),
            ),
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let back: StrongValidator = serde_json::from_str(&json).unwrap();
            assert_eq!(back, v);
        }
        assert_eq!(
            serde_json::to_string(&StrongValidator::ETag(
                StrongEntityTag::parse("\"abc\"").unwrap()
            ))
            .unwrap(),
            "\"etag:\\\"abc\\\"\""
        );
    }
    #[test]
    fn serde_rejects_weak_and_unknown_prefix() {
        assert!(serde_json::from_str::<StrongValidator>("\"etag:W/\\\"a\\\"\"").is_err());
        assert!(serde_json::from_str::<StrongValidator>("\"bogus:x\"").is_err());
    }

    // verify_partial: the §15.3.7 asymmetry
    #[test]
    fn verify_partial_etag_present_and_equal_passes() {
        let v = StrongValidator::ETag(StrongEntityTag::parse("\"a\"").unwrap());
        assert_eq!(v.verify_partial(&headers(&[("etag", "\"a\"")])), Ok(()));
    }
    #[test]
    fn verify_partial_etag_missing_is_a_mismatch() {
        // §15.3.7: a 206 MUST carry ETag if a 200 would have
        let v = StrongValidator::ETag(StrongEntityTag::parse("\"a\"").unwrap());
        assert_eq!(
            v.verify_partial(&HeaderMap::new()),
            Err(ValidatorMismatch::Missing)
        );
    }
    #[test]
    fn verify_partial_etag_different_is_a_mismatch() {
        let v = StrongValidator::ETag(StrongEntityTag::parse("\"a\"").unwrap());
        assert!(matches!(
            v.verify_partial(&headers(&[("etag", "\"b\"")])),
            Err(ValidatorMismatch::Different { .. })
        ));
    }
    #[test]
    fn verify_partial_weak_etag_on_206_is_a_mismatch() {
        let v = StrongValidator::ETag(StrongEntityTag::parse("\"a\"").unwrap());
        assert!(matches!(
            v.verify_partial(&headers(&[("etag", "W/\"a\"")])),
            Err(ValidatorMismatch::Weak(_))
        ));
    }
    #[test]
    fn verify_partial_last_modified_absent_is_ok() {
        // §15.3.7: a 206 to an If-Range request SHOULD NOT repeat non-required headers
        let v = StrongValidator::LastModified(
            ImfFixdate::parse("Sun, 06 Nov 1994 08:49:37 GMT").unwrap(),
        );
        assert_eq!(v.verify_partial(&HeaderMap::new()), Ok(()));
    }
    #[test]
    fn verify_partial_last_modified_different_is_a_mismatch() {
        let v = StrongValidator::LastModified(
            ImfFixdate::parse("Sun, 06 Nov 1994 08:49:37 GMT").unwrap(),
        );
        assert!(matches!(
            v.verify_partial(&headers(&[(
                "last-modified",
                "Mon, 07 Nov 1994 08:49:37 GMT"
            )])),
            Err(ValidatorMismatch::Different { .. })
        ));
    }
}
