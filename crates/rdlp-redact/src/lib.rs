//! Value-level URL credential redaction.
//!
//! Provides [`redact_str`] for stripping sensitive query-parameter values and
//! userinfo credentials from URL strings before they reach log sinks, and the
//! wrapper types [`RedactedUrl`] (borrowed) and [`RedactedUrlBuf`] (owned) whose
//! [`Display`](std::fmt::Display) / [`Debug`](std::fmt::Debug) implementations
//! automatically redact on format.
//!
//! Also provides [`text`], the control/bidi-character filters that make text
//! safe to reach a terminal or log sink (a distinct threat from the
//! credential redaction above — see that module's doc for why it lives here).

use std::borrow::Cow;
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

pub mod text;

/// Ordered set of `(pattern, replacement)` pairs applied left-to-right by [`redact_str`].
///
/// **Ordering rules:**
/// - Exact case-sensitive AWS SigV4 patterns come first. Like the ordering
///   bullet below, this is now belt-and-braces rather than load-bearing: the
///   character before `Signature` in `X-Amz-Signature` is `-`, which is not in
///   the boundary set, so the generic `signature=` pattern cannot match it
///   whatever the order. Kept as a convention, not as a guarantee.
/// - Longer param names precede shorter ones. With the boundary anchor this is
///   now belt-and-braces rather than load-bearing: `_` and `-` are not in the
///   boundary set, so `token=` cannot match inside `access_token=` regardless
///   of order. Kept because the AWS-first rule above it IS load-bearing and the
///   two read as one convention.
/// - Generic query-parameter patterns capture the boundary separator in group 1
///   and the value in group 2; `redact_str`'s closure re-emits the boundary, so
///   the `?`/`&`/space is preserved and the pattern only matches at a real
///   parameter start (not as a suffix inside a longer key like
///   `X-Amz-Signature`).
///   The `^` alternative also matches bare credential fragments with no leading
///   separator (e.g. `token=secret` as a standalone log string).
/// - Userinfo (`//user:pass@host`) is NOT in this array; it has no name=value
///   shape and lives in [`USERINFO_PATTERN`], which carries its own rationale.
///
/// Each entry pairs a pattern with the parameter NAME to re-emit. Every
/// pattern exposes group 1 = the optional boundary character and group 2 = the
/// value; `redact_str` builds the replacement from those, which is what lets
/// the trailing-paren handling live in code rather than in 21 replacement
/// strings.
#[allow(clippy::expect_used)]
static SANITIZE_PATTERNS: LazyLock<[(Regex, &str); 21]> = LazyLock::new(|| {
    [
        // ── AWS SigV4 exact-case — no boundary group needed (unique prefix) ────
        (
            Regex::new(r"()X-Amz-Security-Token=([^&\s]+)").expect("valid regex"),
            "X-Amz-Security-Token",
        ),
        (
            Regex::new(r"()X-Amz-Signature=([^&\s]+)").expect("valid regex"),
            "X-Amz-Signature",
        ),
        (
            Regex::new(r"()X-Amz-Credential=([^&\s]+)").expect("valid regex"),
            "X-Amz-Credential",
        ),
        // ── Longer param names first (boundary-capturing group) ────────────────
        // Pattern: `(^|[?&\s])name=([^&\s]+)` — group 1 the boundary, group 2
        // the value. There is no replacement STRING: `redact_str` builds the
        // output in a closure, which is where the trailing-paren handling and
        // the reason the delimiter is left unmatched are documented.
        //
        // The `(^|[?&\s])` captures the separator (or the empty start-of-string)
        // so that:
        //   • bare fragments like `token=secret` (no leading `?`/`&`) are redacted, and
        //   • a credential named in free text after a space (`failed with
        //     token=SECRET`) is still redacted, and
        //   • hyphenated names like `X-Amz-Signature` are NOT matched by the generic
        //     `signature=` pattern — `-` is not in the boundary set, which is the
        //     property that must survive any widening of it.
        (
            Regex::new(r"(?i)(^|[?&\s])access_token=([^&\s]+)").expect("valid regex"),
            "access_token",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])client_secret=([^&\s]+)").expect("valid regex"),
            "client_secret",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])id_token=([^&\s]+)").expect("valid regex"),
            "id_token",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])api_key=([^&\s]+)").expect("valid regex"),
            "api_key",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])otp_code=([^&\s]+)").expect("valid regex"),
            "otp_code",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])authorization=([^&\s]+)").expect("valid regex"),
            "authorization",
        ),
        // ── Generic shorter names (boundary-capturing group) ───────────────────
        (
            Regex::new(r"(?i)(^|[?&\s])token=([^&\s]+)").expect("valid regex"),
            "token",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])key=([^&\s]+)").expect("valid regex"),
            "key",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])password=([^&\s]+)").expect("valid regex"),
            "password",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])secret=([^&\s]+)").expect("valid regex"),
            "secret",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])bearer=([^&\s]+)").expect("valid regex"),
            "bearer",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])signature=([^&\s]+)").expect("valid regex"),
            "signature",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])sig=([^&\s]+)").expect("valid regex"),
            "sig",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])hmac=([^&\s]+)").expect("valid regex"),
            "hmac",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])auth=([^&\s]+)").expect("valid regex"),
            "auth",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])session=([^&\s]+)").expect("valid regex"),
            "session",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])code=([^&\s]+)").expect("valid regex"),
            "code",
        ),
        (
            Regex::new(r"(?i)(^|[?&\s])otp=([^&\s]+)").expect("valid regex"),
            "otp",
        ),
    ]
});

/// Userinfo credentials in a URL authority (`//user:pass@host`).
///
/// Kept out of [`SANITIZE_PATTERNS`] because it has no `name=value` shape:
/// there is no parameter name to re-emit and no value whose trailing `)` needs
/// giving back, so it uses a plain replacement.
#[allow(clippy::expect_used)]
static USERINFO_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"//[^@\s/]+@").expect("valid regex"));

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
    if let Cow::Owned(replaced) = USERINFO_PATTERN.replace_all(&result, "//*:*@") {
        result = Cow::Owned(replaced);
    }
    for (re, name) in SANITIZE_PATTERNS.iter() {
        let replaced = re.replace_all(&result, |caps: &regex::Captures<'_>| {
            let boundary = caps.get(1).map_or("", |m| m.as_str());
            let value = caps.get(2).map_or("", |m| m.as_str());
            // The value class deliberately ADMITS `)`, because RFC 3986 §2.2
            // makes it a legal unencoded sub-delim — excluding it would stop
            // the match at the first `)` and leave the rest of a secret in the
            // clear. Trailing `)` are given back instead, so the parentheses
            // `wreq::Error` wraps a URI in (`for uri (…)`) survive.
            //
            // The delimiter after the value is NOT part of the match. Consuming
            // it (an earlier attempt) moved the scan cursor past the `&` that
            // the NEXT occurrence of the same parameter needs for its own
            // `(^|[?&\s])` anchor, so `token=a&token=b` redacted only the first.
            let trailing_parens = value.len() - value.trim_end_matches(')').len();
            let mut out = String::with_capacity(boundary.len() + name.len() + 4 + trailing_parens);
            out.push_str(boundary);
            out.push_str(name);
            out.push_str("=***");
            for _ in 0..trailing_parens {
                out.push(')');
            }
            out
        });
        if let Cow::Owned(replaced) = replaced {
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

/// Render a newtype error variant's `Debug` with its free text redacted.
///
/// Two crates hand-write `Debug` for an error enum whose newtype variants
/// carry operator-assembled text; both need the same three lines. The
/// redaction lives here so neither can drift from it.
///
/// ```
/// # use std::fmt;
/// # struct E(String);
/// impl fmt::Debug for E {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         rdlp_redact::redacted_debug_tuple!(f, "E", &self.0)
///     }
/// }
/// ```
#[macro_export]
macro_rules! redacted_debug_tuple {
    ($f:expr, $name:literal, $text:expr) => {
        $f.debug_tuple($name)
            .field(&$crate::redact_str($text))
            .finish()
    };
}

/// Serialize a free-text field with credentials stripped.
///
/// For a type that derives `Serialize`: the derive reads each field directly,
/// so redacting its `Display` and `Debug` does nothing for the serialized form.
/// This guards that form, on the field, so it applies to every construction
/// site rather than to whichever ones a reader remembers to check.
///
/// ```
/// # #[derive(serde::Serialize)]
/// struct Payload {
///     #[serde(serialize_with = "rdlp_redact::serialize_redacted")]
///     message: String,
/// }
/// ```
///
/// # Errors
///
/// Propagates the serializer's own error.
#[cfg(feature = "serde")]
pub fn serialize_redacted<S>(value: &str, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&redact_str(value))
}
