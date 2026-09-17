//! Netscape/Mozilla cookie file loader.
//!
//! Loads the Netscape cookie format used by browsers and tools like `curl`,
//! `wget`, and browser cookie export extensions
//! (`domain\tinclude_subdomains\tpath\tsecure\texpiry\tname\tvalue`, with
//! curl's `#HttpOnly_` line marker) into a [`CookieStore`].
//!
//! Parsing is delegated to [`netscape_cookie_file_parser`], which follows the
//! yt-dlp / `http.cookiejar.MozillaCookieJar` reference semantics: exactly
//! seven fields, a decimal `expiry`, `#HttpOnly_` as cookie metadata rather
//! than a comment, and raw bytes throughout so one non-UTF-8 line cannot
//! abort the whole file.

use std::path::Path;

use log::trace;
use netscape_cookie_file_parser::{Cookie, NetscapeCookieParser, ParseErrorKind};
use wreq::cookie::CookieStore;

use crate::util;

/// Parse a Netscape-format cookie file and insert cookies into the jar.
///
/// Returns the number of cookies successfully loaded.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn load_cookie_file(
    path: &Path,
    jar: &impl CookieStore,
) -> Result<usize, std::io::Error> {
    // Safe: sync cookie helper — async callers wrap in spawn_blocking (see rdlp-cookies/src/lib.rs).
    #[allow(clippy::disallowed_methods)]
    let content = std::fs::read(path)?;
    Ok(load_cookies(&content, jar))
}

/// Parse cookie file content and insert into jar.
///
/// Malformed records are skipped (logged at `trace` by line number only —
/// the line holds cookie values). Returns the number of cookies loaded.
fn load_cookies(content: &[u8], jar: &impl CookieStore) -> usize {
    let mut count = 0;

    for record in NetscapeCookieParser::new(content) {
        let cookie = match record {
            Ok(cookie) => cookie,
            Err(e) => {
                trace!("Skipping malformed cookie line {}: {:?}", e.line, e.kind);
                // A `&[u8]` reader cannot fail, so this is the only `Io` this
                // loop can see; nothing to abort.
                debug_assert!(!matches!(e.kind, ParseErrorKind::Io(_)));
                continue;
            }
        };
        if insert_cookie(&cookie, jar) {
            count += 1;
        }
    }

    count
}

/// Insert a parsed cookie into the wreq jar.
///
/// Empty names are skipped. The parser keeps fields as raw bytes; a cookie
/// with a non-UTF-8 field is skipped rather than decoded lossily, because a
/// `U+FFFD` would be stored and sent as the cookie's value.
fn insert_cookie(cookie: &Cookie, jar: &impl CookieStore) -> bool {
    if cookie.name.is_empty() {
        return false;
    }
    let fields =
        [&cookie.domain, &cookie.name, &cookie.value, &cookie.path].map(|f| str::from_utf8(f));
    let [Ok(domain), Ok(name), Ok(value), Ok(path)] = fields else {
        trace!("Skipping cookie with a non-UTF-8 field");
        return false;
    };
    util::insert_cookie_into_jar(
        jar,
        domain,
        name,
        value,
        path,
        cookie.secure,
        cookie.http_only,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wreq::Uri;

    fn make_jar() -> Arc<wreq::cookie::Jar> {
        Arc::new(wreq::cookie::Jar::default())
    }

    /// Flatten a `Cookies` result to a single string for assertions.
    fn cookies_string(cookies: &wreq::cookie::Cookies) -> Option<String> {
        let parts: Vec<&wreq::header::HeaderValue> = match cookies {
            wreq::cookie::Cookies::Compressed(hv) => vec![hv],
            wreq::cookie::Cookies::Uncompressed(v) => v.iter().collect(),
            _ => return None,
        };
        let joined: Vec<String> = parts
            .into_iter()
            .filter_map(|hv| hv.to_str().ok().map(str::to_owned))
            .collect();
        if joined.is_empty() {
            None
        } else {
            Some(joined.join("; "))
        }
    }

    #[test]
    fn test_parse_basic_cookie() {
        let jar = make_jar();
        let content = ".example.com\tTRUE\t/\tTRUE\t0\tsession\tabc123";
        let count = load_cookies(content.as_bytes(), &*jar);
        assert_eq!(count, 1);

        let uri: Uri = "https://example.com/".parse().unwrap();
        let cookies = jar.cookies(&uri);
        let cookie_str = cookies_string(&cookies).expect("cookies present");
        assert!(cookie_str.contains("session=abc123"));
    }

    #[test]
    fn test_skip_comments_and_empty_lines() {
        let jar = make_jar();
        let content = "# Netscape HTTP Cookie File\n\n# comment\n.example.com\tTRUE\t/\tFALSE\t0\tname\tvalue\n";
        let count = load_cookies(content.as_bytes(), &*jar);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_multiple_cookies() {
        let jar = make_jar();
        let content = "\
.example.com\tTRUE\t/\tTRUE\t0\ta\t1
.example.com\tTRUE\t/\tTRUE\t0\tb\t2
.other.com\tTRUE\t/\tFALSE\t0\tc\t3";
        let count = load_cookies(content.as_bytes(), &*jar);
        assert_eq!(count, 3);
    }

    #[test]
    fn test_malformed_line_skipped() {
        let jar = make_jar();
        let content = "not\tenough\tfields";
        let count = load_cookies(content.as_bytes(), &*jar);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_empty_name_skipped() {
        let jar = make_jar();
        let content = ".example.com\tTRUE\t/\tFALSE\t0\t\tvalue";
        let count = load_cookies(content.as_bytes(), &*jar);
        assert_eq!(count, 0);
    }

    /// A `CookieStore` that records every `Set-Cookie` header it is given,
    /// so attributes the real jar does not echo back (`HttpOnly`) can be
    /// asserted on.
    #[derive(Default)]
    struct RecordingJar(std::sync::Mutex<Vec<String>>);

    impl CookieStore for RecordingJar {
        fn set_cookies(
            &self,
            cookie_headers: &mut dyn Iterator<Item = &wreq::header::HeaderValue>,
            _uri: &Uri,
        ) {
            let mut seen = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            seen.extend(cookie_headers.filter_map(|hv| hv.to_str().ok().map(str::to_owned)));
        }

        fn cookies(&self, _uri: &Uri) -> wreq::cookie::Cookies {
            wreq::cookie::Cookies::Empty
        }
    }

    #[test]
    fn httponly_prefix_sets_the_httponly_attribute() {
        let jar = RecordingJar::default();
        let content = "#HttpOnly_.example.com\tTRUE\t/\tTRUE\t0\tsid\tsecret\n\
                       .example.com\tTRUE\t/\tTRUE\t0\tplain\tvalue";
        assert_eq!(load_cookies(content.as_bytes(), &jar), 2);

        let seen = jar
            .0
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sid = seen
            .iter()
            .find(|h| h.starts_with("sid="))
            .expect("sid stored");
        assert!(sid.contains("; HttpOnly"), "{sid}");
        let plain = seen
            .iter()
            .find(|h| h.starts_with("plain="))
            .expect("plain stored");
        assert!(!plain.contains("HttpOnly"), "{plain}");
    }

    #[test]
    fn a_non_utf8_line_does_not_abort_the_rest_of_the_file() {
        // `read_to_string` used to fail the whole file on one such byte.
        let jar = make_jar();
        let mut content = b".example.com\tTRUE\t/\tTRUE\t0\tfirst\t1\n".to_vec();
        content.extend_from_slice(b".example.com\tTRUE\t/\tTRUE\t0\tlatin\t\xe9\n");
        content.extend_from_slice(b".example.com\tTRUE\t/\tTRUE\t0\tlast\t3\n");

        let count = load_cookies(&content, &*jar);

        // The non-UTF-8 cookie is skipped (not stored lossily); its
        // neighbours still load.
        assert_eq!(count, 2);
        let uri: Uri = "https://example.com/".parse().unwrap();
        let cookie_str = cookies_string(&jar.cookies(&uri)).expect("cookies present");
        assert!(
            cookie_str.contains("first=1") && cookie_str.contains("last=3"),
            "{cookie_str}"
        );
    }

    #[test]
    fn test_httponly_prefix() {
        let jar = make_jar();
        // Some exporters use #HttpOnly_ prefix on the domain
        let content = "#HttpOnly_.example.com\tTRUE\t/\tTRUE\t0\thttponly_cookie\tsecret";
        let count = load_cookies(content.as_bytes(), &*jar);
        assert_eq!(count, 1);

        let uri: Uri = "https://example.com/".parse().unwrap();
        let cookies = jar.cookies(&uri);
        let cookie_str = cookies_string(&cookies).expect("cookies present");
        assert!(cookie_str.contains("httponly_cookie=secret"));
    }
}
