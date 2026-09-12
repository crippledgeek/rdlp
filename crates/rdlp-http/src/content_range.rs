//! The one `Content-Range` grammar (RFC 9110 §14.4).
//!
//! Read by the probe and by every ranged download path, so a length the
//! probe reports and a length a 206 is verified against can never disagree
//! about what counts as a valid field value.

use wreq::header::HeaderMap;

/// A parsed, validated single-part `Content-Range` response header.
///
/// Grammar (RFC 9110 §14.4):
///
/// ```text
/// Content-Range = range-unit SP ( range-resp / unsatisfied-range )
/// range-resp    = incl-range "/" ( complete-length / "*" )
/// incl-range    = first-pos "-" last-pos
/// ```
///
/// Both positions are INCLUSIVE, so the span covers
/// `last_pos - first_pos + 1` bytes.
///
/// Only the `bytes` range unit is represented: §14.4 requires that a recipient
/// which does not understand the unit "MUST NOT attempt to recombine it with a
/// stored representation", and recombining is exactly what the chunk merge
/// does. `unsatisfied-range` (`bytes */1234`, sent with 416) is likewise not
/// represented here — it describes no enclosed content — and is read through
/// [`Self::parse_unsatisfied`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    /// First byte position of the enclosed span (inclusive).
    first_pos: u64,
    /// Last byte position of the enclosed span (inclusive).
    last_pos: u64,
    /// Total length of the selected representation; `None` when the server
    /// sent `*` to signal that the complete length was unknown (§14.4).
    complete_length: Option<u64>,
}

impl ContentRange {
    /// Parse a `Content-Range` field value, returning `None` when it is
    /// malformed, carries a non-`bytes` unit, or is *invalid* per RFC 9110
    /// §14.4 — that is, `last-pos < first-pos`, or a `complete-length` less
    /// than or equal to `last-pos`. The spec's directive for an invalid value
    /// is that the recipient "MUST NOT attempt to recombine the received
    /// content with a stored representation", so an unparseable or invalid
    /// header and a missing one are treated alike: the response is not usable.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let (unit, rest) = value.trim().split_once(' ')?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return None;
        }

        let (incl_range, complete) = rest.trim().split_once('/')?;
        let (first, last) = incl_range.split_once('-')?;

        let first_pos: u64 = first.trim().parse().ok()?;
        let last_pos: u64 = last.trim().parse().ok()?;

        // §14.4: a last-pos below first-pos makes the field value invalid.
        if last_pos < first_pos {
            return None;
        }

        let complete_length = match complete.trim() {
            "*" => None,
            digits => {
                let total: u64 = digits.parse().ok()?;
                // §14.4: a complete-length <= last-pos makes the value invalid.
                if total <= last_pos {
                    return None;
                }
                Some(total)
            }
        };

        Some(Self {
            first_pos,
            last_pos,
            complete_length,
        })
    }

    /// Read the header map and parse the `Content-Range` field if present.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        headers
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(Self::parse)
    }

    /// First byte position of the enclosed span (inclusive).
    #[must_use]
    pub const fn first_pos(self) -> u64 {
        self.first_pos
    }

    /// Last byte position of the enclosed span (inclusive).
    #[must_use]
    pub const fn last_pos(self) -> u64 {
        self.last_pos
    }

    /// The representation's complete length; `None` when the server sent `*`
    /// (§14.4: "the representation length was unknown").
    #[must_use]
    pub const fn complete_length(self) -> Option<u64> {
        self.complete_length
    }

    /// `bytes */complete-length` (§14.4 `unsatisfied-range`), as sent with a 416.
    #[must_use]
    pub fn parse_unsatisfied(value: &str) -> Option<u64> {
        let (unit, rest) = value.trim().split_once(' ')?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return None;
        }
        rest.trim().strip_prefix("*/")?.trim().parse().ok()
    }

    /// Read the header map and parse the `Content-Range` field's
    /// `unsatisfied-range` form if present (only sent with a 416).
    #[must_use]
    pub fn unsatisfied_from_headers(headers: &HeaderMap) -> Option<u64> {
        headers
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(Self::parse_unsatisfied)
    }
}

#[cfg(test)]
mod tests {
    //! `ContentRange::parse` grammar branches (RFC 9110 §14.4).
    //!
    //! The chunk path decides whether a response may be written into its
    //! offset slot from this parser's verdict, and the resume path compares
    //! its sidecar against the length it reports, so each accept/reject
    //! branch is pinned individually rather than only through mockito flows.
    use super::*;

    fn make_headers(content_range: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(cr) = content_range {
            h.insert("content-range", cr.parse().unwrap());
        }
        h
    }

    #[test]
    fn from_headers_reads_the_complete_length_of_a_206() {
        let h = make_headers(Some("bytes 0-262143/1048576"));
        assert_eq!(
            ContentRange::from_headers(&h).and_then(ContentRange::complete_length),
            Some(1_048_576)
        );
        assert_eq!(ContentRange::from_headers(&make_headers(None)), None);
    }

    #[test]
    fn parses_a_conformant_single_part_range() {
        let range = ContentRange::parse("bytes 0-1023/2048").expect("valid range must parse");
        assert_eq!(range.first_pos, 0);
        assert_eq!(range.last_pos, 1023);
        assert_eq!(range.complete_length, Some(2048));
    }

    /// The accessors are what `range_verdict` reads a `Partial` verdict's
    /// `ContentRange` through — pinned directly so a field rename cannot
    /// silently desync them from the private fields covered by name above.
    #[test]
    fn accessors_read_the_same_fields_as_direct_access() {
        let range = ContentRange::parse("bytes 0-1023/2048").expect("valid range must parse");
        assert_eq!(range.first_pos(), range.first_pos);
        assert_eq!(range.last_pos(), range.last_pos);
        assert_eq!(range.complete_length(), range.complete_length);
    }

    /// `*` for complete-length is explicitly legal — "An asterisk character
    /// ("*") in place of the complete-length indicates that the representation
    /// length was unknown when the header field was generated" (§14.4). The
    /// span is still fully usable, so this MUST parse; rejecting it would
    /// break every server that cannot cheaply determine total length.
    #[test]
    fn accepts_unknown_complete_length() {
        let range =
            ContentRange::parse("bytes 0-1023/*").expect("unknown complete-length is legal");
        assert_eq!(range.first_pos, 0);
        assert_eq!(range.last_pos, 1023);
        assert_eq!(range.complete_length, None);
    }

    /// §14.4: a recipient that does not understand the range unit "MUST NOT
    /// attempt to recombine it with a stored representation" — and recombining
    /// is exactly what the chunk merge does.
    #[test]
    fn rejects_unknown_range_unit() {
        assert!(ContentRange::parse("items 0-1023/2048").is_none());
        assert!(ContentRange::parse("seconds 0-10/60").is_none());
    }

    #[test]
    fn accepts_case_insensitive_unit() {
        assert!(ContentRange::parse("BYTES 0-1023/2048").is_some());
    }

    /// §14.4 invalidity: "a last-pos value less than its first-pos value".
    #[test]
    fn rejects_last_pos_below_first_pos() {
        assert!(ContentRange::parse("bytes 1023-0/2048").is_none());
    }

    /// §14.4 invalidity: "a complete-length value less than or equal to its
    /// last-pos value". Positions are inclusive, so a 0-1023 span needs at
    /// least 1024 total bytes; 1024 is the tightest legal value.
    #[test]
    fn rejects_complete_length_not_above_last_pos() {
        assert!(ContentRange::parse("bytes 0-1023/1023").is_none());
        assert!(ContentRange::parse("bytes 0-1023/1024").is_some());
    }

    /// A single-position span is legal and covers exactly one byte.
    #[test]
    fn accepts_single_byte_span() {
        let range = ContentRange::parse("bytes 5-5/10").expect("single-byte span is valid");
        assert_eq!(range.first_pos, range.last_pos);
    }

    #[test]
    fn rejects_unsatisfied_range_form() {
        // `bytes */1234` accompanies a 416 and encloses no content.
        assert!(ContentRange::parse("bytes */1234").is_none());
    }

    #[test]
    fn rejects_structurally_malformed_values() {
        assert!(ContentRange::parse("").is_none());
        assert!(ContentRange::parse("bytes").is_none());
        assert!(ContentRange::parse("bytes 0-1023").is_none());
        assert!(ContentRange::parse("bytes abc-def/2048").is_none());
        assert!(ContentRange::parse("bytes 0-1023/notanumber").is_none());
        assert!(ContentRange::parse("0-1023/2048").is_none());
    }

    #[test]
    fn parse_unsatisfied_reads_only_the_star_form_in_bytes() {
        assert_eq!(ContentRange::parse_unsatisfied("bytes */1234"), Some(1234));
        assert_eq!(ContentRange::parse_unsatisfied("bytes 0-1/1234"), None);
        assert_eq!(ContentRange::parse_unsatisfied("items */5"), None);
        assert_eq!(ContentRange::parse_unsatisfied("bytes */*"), None);
        assert_eq!(
            ContentRange::unsatisfied_from_headers(&make_headers(Some("bytes */7"))),
            Some(7)
        );
        assert_eq!(
            ContentRange::unsatisfied_from_headers(&make_headers(None)),
            None
        );
    }
}
