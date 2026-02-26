//! Netscape/Mozilla cookie file parser.
//!
//! Parses the standard Netscape cookie format used by browsers and tools
//! like `curl`, `wget`, and browser cookie export extensions.
//!
//! Format: `domain\tinclude_subdomains\tpath\tsecure\texpiry\tname\tvalue`

use std::path::Path;

use log::trace;
use reqwest::cookie::CookieStore;

use crate::util;

/// A parsed cookie from a Netscape cookie file.
#[derive(Debug)]
struct NetscapeCookie {
    domain: String,
    _include_subdomains: bool,
    path: String,
    secure: bool,
    _expiry: u64,
    name: String,
    value: String,
}

/// Parse a Netscape-format cookie file and insert cookies into the jar.
///
/// Returns the number of cookies successfully loaded.
pub(crate) fn load_cookie_file(
    path: &Path,
    jar: &impl CookieStore,
) -> Result<usize, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    Ok(load_cookie_string(&content, jar))
}

/// Parse cookie string content and insert into jar.
///
/// Returns the number of cookies loaded.
fn load_cookie_string(content: &str, jar: &impl CookieStore) -> usize {
    let mut count = 0;

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines
        if line.is_empty() {
            continue;
        }

        // Handle #HttpOnly_ prefix (valid cookie with httponly flag)
        let line = if let Some(stripped) = line.strip_prefix("#HttpOnly_") {
            stripped
        } else if line.starts_with('#') {
            // Regular comment
            continue;
        } else {
            line
        };

        let Some(cookie) = parse_cookie_line(line) else {
            trace!("Skipping malformed cookie line: {line}");
            continue;
        };
        if insert_cookie(&cookie, jar) {
            count += 1;
        }
    }

    count
}

/// Parse a single Netscape cookie line.
fn parse_cookie_line(line: &str) -> Option<NetscapeCookie> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 7 {
        return None;
    }

    let domain = fields[0].to_string();
    let include_subdomains = fields[1].eq_ignore_ascii_case("TRUE");
    let path = fields[2].to_string();
    let secure = fields[3].eq_ignore_ascii_case("TRUE");
    let expiry = fields[4].parse().unwrap_or(0);
    let name = fields[5].to_string();
    let value = fields[6].to_string();

    // Skip empty names
    if name.is_empty() {
        return None;
    }

    Some(NetscapeCookie {
        domain,
        _include_subdomains: include_subdomains,
        path,
        secure,
        _expiry: expiry,
        name,
        value,
    })
}

/// Insert a parsed cookie into the reqwest jar.
fn insert_cookie(cookie: &NetscapeCookie, jar: &impl CookieStore) -> bool {
    util::insert_cookie_into_jar(
        jar,
        &cookie.domain,
        &cookie.name,
        &cookie.value,
        &cookie.path,
        cookie.secure,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use url::Url;

    fn make_jar() -> Arc<reqwest::cookie::Jar> {
        Arc::new(reqwest::cookie::Jar::default())
    }

    #[test]
    fn test_parse_basic_cookie() {
        let jar = make_jar();
        let content = ".example.com\tTRUE\t/\tTRUE\t0\tsession\tabc123";
        let count = load_cookie_string(content, &*jar);
        assert_eq!(count, 1);

        let url = Url::parse("https://example.com/").unwrap();
        let cookies = jar.cookies(&url);
        assert!(cookies.is_some());
        let cookie_str = cookies.unwrap();
        assert!(cookie_str.to_str().unwrap().contains("session=abc123"));
    }

    #[test]
    fn test_skip_comments_and_empty_lines() {
        let jar = make_jar();
        let content = "# Netscape HTTP Cookie File\n\n# comment\n.example.com\tTRUE\t/\tFALSE\t0\tname\tvalue\n";
        let count = load_cookie_string(content, &*jar);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_multiple_cookies() {
        let jar = make_jar();
        let content = "\
.example.com\tTRUE\t/\tTRUE\t0\ta\t1
.example.com\tTRUE\t/\tTRUE\t0\tb\t2
.other.com\tTRUE\t/\tFALSE\t0\tc\t3";
        let count = load_cookie_string(content, &*jar);
        assert_eq!(count, 3);
    }

    #[test]
    fn test_malformed_line_skipped() {
        let jar = make_jar();
        let content = "not\tenough\tfields";
        let count = load_cookie_string(content, &*jar);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_empty_name_skipped() {
        let jar = make_jar();
        let content = ".example.com\tTRUE\t/\tFALSE\t0\t\tvalue";
        let count = load_cookie_string(content, &*jar);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_httponly_prefix() {
        let jar = make_jar();
        // Some exporters use #HttpOnly_ prefix on the domain
        let content = "#HttpOnly_.example.com\tTRUE\t/\tTRUE\t0\thttponly_cookie\tsecret";
        let count = load_cookie_string(content, &*jar);
        assert_eq!(count, 1);

        let url = Url::parse("https://example.com/").unwrap();
        let cookies = jar.cookies(&url);
        assert!(cookies.is_some());
        assert!(
            cookies
                .unwrap()
                .to_str()
                .unwrap()
                .contains("httponly_cookie=secret")
        );
    }
}
