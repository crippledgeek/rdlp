//! Value-level URL credential redaction.
//!
//! Provides [`redact_str`] for stripping sensitive query-parameter values and
//! userinfo credentials from URL strings before they reach log sinks, and the
//! wrapper types [`RedactedUrl`] (borrowed) and [`RedactedUrlBuf`] (owned) whose
//! [`Display`](std::fmt::Display) / [`Debug`](std::fmt::Debug) implementations
//! automatically redact on format.

use std::borrow::Cow;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

/// Ordered set of `(pattern, replacement)` pairs applied left-to-right by [`redact_str`].
///
/// **Ordering rules:**
/// - Exact case-sensitive AWS SigV4 patterns come first since they have
///   upper-case letters that would be clobbered by the generic case-insensitive
///   `signature` / `sig` patterns.
/// - Longer param names precede shorter to avoid substring shadowing
///   (`access_token` before `token`, `otp_code` before `otp`).
/// - Generic query-parameter patterns use a `(^|[?&])` capture group for the
///   boundary separator and re-emit it via `${1}` in the replacement, so that
///   the `?`/`&` is preserved but the pattern only matches at a real parameter
///   start (not as a suffix inside a longer key like `X-Amz-Signature`).
///   The `^` alternative also matches bare credential fragments with no leading
///   separator (e.g. `token=secret` as a standalone log string).
/// - The userinfo (`//user:pass@host`) pattern is a standalone structural rule.
#[allow(clippy::expect_used)]
static SANITIZE_PATTERNS: LazyLock<[(Regex, &str); 22]> = LazyLock::new(|| {
    [
        // ── AWS SigV4 exact-case — no boundary group needed (unique prefix) ────
        (
            Regex::new(r"X-Amz-Security-Token=[^&\s)]+").expect("valid regex"),
            "X-Amz-Security-Token=***",
        ),
        (
            Regex::new(r"X-Amz-Signature=[^&\s)]+").expect("valid regex"),
            "X-Amz-Signature=***",
        ),
        (
            Regex::new(r"X-Amz-Credential=[^&\s)]+").expect("valid regex"),
            "X-Amz-Credential=***",
        ),
        // ── Longer param names first (boundary-capturing group) ────────────────
        // Pattern: `(^|[?&])name=[^&\s)]+`  →  replacement: `${1}name=***`
        // The `(^|[?&])` captures the separator (or the empty start-of-string) and
        // re-emits it via `${1}`, so that:
        //   • bare fragments like `token=secret` (no leading `?`/`&`) are redacted, and
        //   • hyphenated names like `X-Amz-Signature` are NOT matched by the generic
        //     `signature=` pattern (the `-` before it is neither `^` nor `?`/`&`).
        (
            Regex::new(r"(?i)(^|[?&])access_token=[^&\s)]+").expect("valid regex"),
            "${1}access_token=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])client_secret=[^&\s)]+").expect("valid regex"),
            "${1}client_secret=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])id_token=[^&\s)]+").expect("valid regex"),
            "${1}id_token=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])api_key=[^&\s)]+").expect("valid regex"),
            "${1}api_key=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])otp_code=[^&\s)]+").expect("valid regex"),
            "${1}otp_code=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])authorization=[^&\s)]+").expect("valid regex"),
            "${1}authorization=***",
        ),
        // ── Generic shorter names (boundary-capturing group) ───────────────────
        (
            Regex::new(r"(?i)(^|[?&])token=[^&\s)]+").expect("valid regex"),
            "${1}token=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])key=[^&\s)]+").expect("valid regex"),
            "${1}key=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])password=[^&\s)]+").expect("valid regex"),
            "${1}password=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])secret=[^&\s)]+").expect("valid regex"),
            "${1}secret=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])bearer=[^&\s)]+").expect("valid regex"),
            "${1}bearer=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])signature=[^&\s)]+").expect("valid regex"),
            "${1}signature=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])sig=[^&\s)]+").expect("valid regex"),
            "${1}sig=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])hmac=[^&\s)]+").expect("valid regex"),
            "${1}hmac=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])auth=[^&\s)]+").expect("valid regex"),
            "${1}auth=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])session=[^&\s)]+").expect("valid regex"),
            "${1}session=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])code=[^&\s)]+").expect("valid regex"),
            "${1}code=***",
        ),
        (
            Regex::new(r"(?i)(^|[?&])otp=[^&\s)]+").expect("valid regex"),
            "${1}otp=***",
        ),
        // ── Userinfo credentials in URL authority (`//user:pass@host`) ─────────
        (Regex::new(r"//[^@\s/]+@").expect("valid regex"), "//*:*@"),
    ]
});

/// Redact credential-bearing query parameters and userinfo from a URL string.
///
/// Applies all patterns in `SANITIZE_PATTERNS` in order. Non-sensitive
/// parameters (e.g. `range`, `quality`, `X-Amz-Date`) are left untouched.
///
/// # Examples
///
/// ```
/// use rdlp_redact::redact_str;
/// assert_eq!(redact_str("https://a:b@host/p?token=secret"), "https://*:*@host/p?token=***");
/// ```
#[must_use]
pub fn redact_str(s: &str) -> String {
    let mut result = Cow::Borrowed(s);
    for (re, replacement) in SANITIZE_PATTERNS.iter() {
        if let Cow::Owned(replaced) = re.replace_all(&result, *replacement) {
            result = Cow::Owned(replaced);
        }
    }
    result.into_owned()
}

/// Generate the redacting `Display` / `Debug` / (feature-gated) `ToValue`
/// impls for a URL wrapper type.
///
/// Both wrappers ([`RedactedUrl`], [`RedactedUrlBuf`]) share one source of
/// truth for the redaction wiring: `Display` always routes the raw value
/// (`self.expose()`) through [`redact_str`] — no wrapper may bypass redaction —
/// `Debug` delegates to `Display`, and `ToValue` renders via `from_display`.
/// The single `expose() -> &str` accessor is what lets one macro cover both the
/// borrowed (`&str`) and owned (`String`) representations.
macro_rules! impl_redacting_traits {
    ($ty:ty) => {
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&redact_str(self.expose()))
            }
        }

        impl fmt::Debug for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        #[cfg(feature = "log-kv")]
        impl log::kv::ToValue for $ty {
            fn to_value(&self) -> log::kv::Value<'_> {
                log::kv::Value::from_display(self)
            }
        }
    };
}

/// Borrowed wrapper around a URL string whose [`Display`](fmt::Display) and
/// [`Debug`](fmt::Debug) automatically redact credentials.
///
/// Use this when you have a `&str` or `&String` that you want to log safely
/// without allocating unless formatting actually occurs.
#[derive(Clone, Copy)]
pub struct RedactedUrl<'a>(&'a str);

impl<'a> RedactedUrl<'a> {
    /// Wrap a URL string slice.
    #[must_use]
    pub fn new<S: AsRef<str> + ?Sized>(url: &'a S) -> Self {
        Self(url.as_ref())
    }

    /// Return the original, unredacted URL.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0
    }
}

impl_redacting_traits!(RedactedUrl<'_>);

/// Owned wrapper around a URL string whose [`Display`](fmt::Display) and
/// [`Debug`](fmt::Debug) automatically redact credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedUrlBuf(String);

impl RedactedUrlBuf {
    /// Wrap an owned URL string.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// Return the original, unredacted URL.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for RedactedUrlBuf {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RedactedUrlBuf {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl_redacting_traits!(RedactedUrlBuf);

#[cfg(test)]
mod tests;

#[cfg(test)]
mod paren_bounded_tests {
    use super::redact_str;

    #[test]
    fn a_credential_parameter_inside_parentheses_keeps_its_closing_paren() {
        // `wreq::Error`'s Display renders `for uri (<url>)`, so a credential
        // in the LAST query parameter sits immediately before `)`. With the
        // value class unbounded on the right the match swallowed the paren,
        // truncating the message. Redaction was still correct; the output was
        // malformed.
        let out = redact_str("for uri (https://cdn.example.com/v.mp4?token=abc123)");
        assert_eq!(out, "for uri (https://cdn.example.com/v.mp4?token=***)");
    }

    #[test]
    fn userinfo_and_a_query_credential_are_both_stripped_in_one_pass() {
        let out = redact_str("for uri (https://u:p@cdn.example.com/v?token=abc123)");
        assert_eq!(out, "for uri (https://*:*@cdn.example.com/v?token=***)");
    }
}
