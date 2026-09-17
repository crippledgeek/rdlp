//! Percent-encoding for URL query values — the one place rdlp decides which
//! octets are escaped, so extractors and the plugin host cannot drift apart.
//!
//! Built on [`percent_encoding`] (the crate `url` itself uses); no bespoke
//! escaping tables.

use std::borrow::Cow;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

/// Octets escaped in a query value: everything except RFC 3986 §2.3
/// unreserved characters (`A-Z a-z 0-9 - . _ ~`), which have no reserved
/// purpose anywhere in a URI and need never be escaped.
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encode `value` for use as a query-string value (or form field).
///
/// Non-ASCII input is escaped byte-wise as UTF-8. Borrows when nothing needs
/// escaping.
#[must_use]
pub fn percent_encode_query_value(value: &str) -> Cow<'_, str> {
    utf8_percent_encode(value, QUERY_VALUE).into()
}

/// Percent-decode `text`, returning `None` when the decoded bytes are not
/// valid UTF-8. `+` is left as-is (it is not a space outside
/// `application/x-www-form-urlencoded`). Borrows when nothing was escaped.
#[must_use]
pub fn percent_decode(text: &str) -> Option<Cow<'_, str>> {
    percent_decode_str(text).decode_utf8().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_keeps_rfc3986_unreserved_and_escapes_the_rest() {
        assert_eq!(
            percent_encode_query_value("Az09-._~ /?&=+#"),
            "Az09-._~%20%2F%3F%26%3D%2B%23"
        );
    }

    #[test]
    fn encode_escapes_non_ascii_bytewise_as_utf8() {
        assert_eq!(percent_encode_query_value("é"), "%C3%A9");
    }

    #[test]
    fn encode_borrows_when_nothing_needs_escaping() {
        assert!(matches!(
            percent_encode_query_value("plain-value_1.0~"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn decode_round_trips_and_leaves_plus_alone() {
        assert_eq!(percent_decode("a%20b+c").as_deref(), Some("a b+c"));
        assert_eq!(
            percent_decode(&percent_encode_query_value("é /?")).as_deref(),
            Some("é /?")
        );
    }

    #[test]
    fn decode_rejects_invalid_utf8() {
        assert_eq!(percent_decode("%FF"), None);
    }
}
