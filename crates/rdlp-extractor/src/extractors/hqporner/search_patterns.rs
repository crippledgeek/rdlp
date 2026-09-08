//! URL builders for HQPorner search and listing pages.

use lazy_regex::{Lazy, Regex, lazy_regex};
use log::warn;
use url::form_urlencoded;

use crate::base::common::resolve_card_url;

/// HQPorner search base URL.
const SEARCH_BASE: &str = "https://hqporner.com/";

/// Pattern to extract the "Next" page URL from pagination.
static NEXT_PAGE_PATTERN: Lazy<Regex> =
    lazy_regex!(r#"href="([^"]+)"[^>]*class="[^"]*pagi-btn[^"]*">Next"#);

/// Build a search URL.
///
/// # Arguments
/// * `query` - Search keyword string.
/// * `page` - 1-based page number.
pub(crate) fn build_search_url(query: &str, page: u32) -> String {
    let encoded: String = form_urlencoded::byte_serialize(query.as_bytes()).collect();
    if page <= 1 {
        format!("{SEARCH_BASE}?q={encoded}")
    } else {
        format!("{SEARCH_BASE}?q={encoded}&p={page}")
    }
}

/// Extract the next listing page URL from pagination HTML.
///
/// Parses the pagination links to find the "Next" page href and resolves it
/// against `page_url` — **the URL that served this listing**, not the site
/// root. `patterns::is_suitable` admits `hqporner.com`, `www.hqporner.com` and
/// `m.hqporner.com`, so resolving against a hardcoded apex would refuse a
/// perfectly ordinary `https://m.hqporner.com/category/x/2` as off-origin and
/// truncate the playlist at page 1 while still returning `Ok`. Passing the
/// page's own URL is the xhamster precedent that [`resolve_card_url`]'s doc
/// comment cites, and it also gives a path-relative href (`3`) the right base.
///
/// A consequence worth stating: a listing on one mirror is refused an absolute
/// Next href pointing at another (`m.` → apex). Every Next href evidenced in
/// tree is root-relative, so nothing legitimate is refused today. If hqporner
/// is ever observed emitting cross-mirror absolute links, the fix is a
/// site-scoped host allowlist — NOT a wider base. Comparing [`Url::origin`] is
/// what buys the port check and the `https`→`http` downgrade refusal for free,
/// and a host-only or same-site comparison would give both up.
///
/// Returns `None` when there is no further page. That covers two cases
/// deliberately, because the caller does the same thing with both — stop
/// paginating and return what it already has:
///
/// * no "Next" link on the page (the ordinary end of a listing), and
/// * a "Next" link whose resolved origin is not the serving page's (#665).
///
/// Same-origin is the right policy here and is stricter in consequence than it
/// is for a search card: this value is fetched by `extract_playlist` with **no
/// operator action at all**, so an off-origin href in pagination markup is a
/// direct server-side fetch of an attacker-chosen host, where a poisoned card
/// still requires someone to click it. Real pagination is same-origin by
/// construction — the site's own links are `/?q=…&p=N` — so nothing legitimate
/// is refused.
///
/// The rejection is logged rather than folded silently into the end-of-listing
/// case, since the two are the same value but not the same event. Returning
/// `Option` rather than the previous `String`-with-`unwrap_or_default` is what
/// keeps "there is no next page" from being spelled as an empty URL.
pub(crate) fn next_listing_page_url(webpage: &str, page_url: &str) -> Option<String> {
    let href = NEXT_PAGE_PATTERN.captures(webpage)?.get(1)?.as_str();
    let resolved = resolve_card_url(page_url, href);
    if resolved.is_none() {
        warn!(
            "[HQPorner] Ignoring off-origin pagination href {}",
            rejected_href_for_log(href)
        );
    }
    resolved
}

/// Render a rejected pagination href for the operator's log.
///
/// Two filters, in this order, because they answer different threats and
/// neither subsumes the other: `sanitize_for_terminal` drops control and bidi
/// code points, which the `([^"]+)` capture in [`NEXT_PAGE_PATTERN`] otherwise
/// admits straight into the log line — and this branch fires precisely when the
/// page is hostile. `RedactedUrl` then masks any credentials, and does *not*
/// touch control characters, which is why the first step is not redundant.
///
/// **The order is load-bearing, not stylistic — it closes a control-character
/// redaction bypass.** Every pattern in `redact_str` anchors on a contiguous
/// literal, and a control character embedded in that literal defeats the
/// anchor; stripping the controls FIRST rejoins it, so the redactor sees what
/// it is looking for. Two measured inputs, each separating the two orders:
///
/// ```text
/// href                                  sanitize→redact (chosen)             redact→sanitize
/// https://evil.test/?to\rken=SECRET&p=3 https://evil.test/?token=***&p=3     ...?token=SECRET&p=3   LEAKS
/// //us\ter:pw@evil.test/?p=3            //*:*@evil.test/?p=3                 //user:pw@evil.test/…  LEAKS
/// ```
///
/// The first is a CR inside a parameter NAME defeating the `token=` anchor;
/// the second is a TAB inside the userinfo, which the `//[^@\s/]+@` pattern
/// cannot match because its class excludes whitespace. Note an ESC exposes
/// neither: it is a control character but not `\s`, and it is not inside the
/// anchor literal — so a test built on ESC alone would stay green with the
/// order swapped. Do not "simplify" these two steps into either sequence
/// without re-running `a_credential_survives_neither_filter_in_this_order`.
///
/// Split out from the `warn!` so the composed result is assertable; a log macro
/// argument is not.
fn rejected_href_for_log(href: &str) -> String {
    let inert = rdlp_redact::text::sanitize_for_terminal(href);
    rdlp_redact::RedactedUrl::new(&inert).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url_page_1() {
        let url = build_search_url("massage", 1);
        assert_eq!(url, "https://hqporner.com/?q=massage");
    }

    #[test]
    fn test_build_search_url_page_2() {
        let url = build_search_url("massage", 2);
        assert_eq!(url, "https://hqporner.com/?q=massage&p=2");
    }

    #[test]
    fn test_build_search_url_encodes_spaces() {
        let url = build_search_url("big tits", 1);
        assert!(url.contains("big+tits") || url.contains("big%20tits"));
    }

    /// The apex listing page, used as the resolution base by every test that
    /// is not specifically about a mirror host.
    const APEX_PAGE: &str = "https://hqporner.com/category/amateur";

    fn host_of(url: &str) -> String {
        url::Url::parse(url)
            .expect("an accepted next-page URL must parse")
            .host_str()
            .expect("an accepted next-page URL must carry a host")
            .to_string()
    }

    fn next_link(href: &str) -> String {
        format!(r#"<a href="{href}" class="pagi-btn">Next</a>"#)
    }

    #[test]
    fn test_next_listing_page_url() {
        let html = r#"<a href="/?q=massage&p=3" class="button mobile-pagi pagi-btn">Next</a>"#;
        let next = next_listing_page_url(html, APEX_PAGE);
        assert_eq!(next.as_deref(), Some("https://hqporner.com/?q=massage&p=3"));
    }

    #[test]
    fn test_next_listing_page_url_category() {
        let html = r#"<a href="/category/amateur/3" class="button mobile-hide pagi-btn">Next</a>"#;
        let next = next_listing_page_url(html, APEX_PAGE);
        assert_eq!(
            next.as_deref(),
            Some("https://hqporner.com/category/amateur/3")
        );
    }

    /// `patterns::is_suitable` admits `m.` and `www.` hosts, so a listing
    /// served by one of them must paginate against ITSELF. Resolving against a
    /// hardcoded apex would read this ordinary same-site href as off-origin and
    /// truncate the playlist at page 1 while still returning `Ok`.
    #[test]
    fn a_mirror_host_paginates_against_its_own_origin() {
        let page = "https://m.hqporner.com/category/x";
        let absolute = next_link("https://m.hqporner.com/category/x/2");
        assert_eq!(
            next_listing_page_url(&absolute, page).as_deref(),
            Some("https://m.hqporner.com/category/x/2")
        );
        // A path-relative href gets the listing's path as its base, too.
        assert_eq!(
            next_listing_page_url(&next_link("3"), page).as_deref(),
            Some("https://m.hqporner.com/category/3")
        );
        // The apex is a DIFFERENT origin from the mirror, and is refused: the
        // check is same-origin-as-the-serving-page, not same-site.
        assert_eq!(
            next_listing_page_url(&next_link("https://hqporner.com/category/x/2"), page),
            None
        );
    }

    /// The two #665 shapes that CAN carry an authority. The first reached the
    /// crawler verbatim through the old `starts_with("http")` arm; the second
    /// became an on-origin path with the concatenation and is refused here
    /// because RFC 3986 resolution reads it as protocol-relative.
    #[test]
    fn next_page_url_cannot_move_the_authority_off_hqporner() {
        for hostile in ["https://evil.test/?q=x&p=3", "//evil.test/?q=x&p=3"] {
            assert_eq!(
                next_listing_page_url(&next_link(hostile), APEX_PAGE),
                None,
                "href {hostile:?} must stop pagination rather than be fetched"
            );
        }
    }

    /// The two #665 shapes that only LOOK hostile: reference resolution turns
    /// both into on-origin paths, so pagination continues. Asserted on the
    /// parsed host, which is what distinguishes them from the concatenation
    /// (`https://hqporner.com` + `@evil.test/…` had host `evil.test`).
    #[test]
    fn next_page_userinfo_and_lookalike_hrefs_stay_on_origin() {
        for href in ["@evil.test/?p=3", ".evil.test/?p=3"] {
            let next = next_listing_page_url(&next_link(href), APEX_PAGE)
                .expect("an on-origin href must be kept");
            assert_eq!(host_of(&next), "hqporner.com", "href {href:?} -> {next}");
        }
    }

    /// The `([^"]+)` capture admits ESC, BEL and friends, and this log line
    /// fires exactly when the page is hostile. `RedactedUrl` masks credentials
    /// and leaves control characters alone, so the sanitize step is what makes
    /// the rendered line inert.
    #[test]
    fn a_rejected_href_logs_inert() {
        let hostile = "https://evil.test/\u{1b}[31mHACKED\u{7}/?p=3";
        assert_eq!(
            next_listing_page_url(&next_link(hostile), APEX_PAGE),
            None,
            "the hostile href must still be refused"
        );
        let logged = rejected_href_for_log(hostile);
        assert!(
            !logged.chars().any(char::is_control),
            "no control character may reach the log: {logged:?}"
        );
        assert_eq!(logged, "https://evil.test/[31mHACKED/?p=3");
    }

    /// The sanitize-then-redact ORDER, which the test above cannot see: its
    /// href carries no credential, so a swapped order stays green there.
    ///
    /// Two discriminating inputs, both of which hide a secret from
    /// `redact_str` by splitting the literal it anchors on, and both of which
    /// sanitizing first puts back together. An ESC would NOT discriminate — it
    /// is a control character but not `\s` and not inside an anchor literal, so
    /// the patterns match with or without sanitizing.
    /// TAB inside the userinfo: `//[^@\s/]+@` excludes whitespace, so redacting
    /// first finds no authority at all and the password reaches the log in
    /// clear once the tab is stripped afterwards.
    #[test]
    fn a_credential_survives_neither_filter_in_this_order() {
        let logged = rejected_href_for_log("//us\u{9}er:pw@evil.test/?p=3");
        assert!(
            logged.contains("*:*@"),
            "the userinfo must be masked: {logged:?}"
        );
        assert!(!logged.contains("pw"), "no credential may reach the log");
        assert!(!logged.chars().any(char::is_control));
    }

    /// CR inside a parameter NAME defeats `redact_str`'s `token=` anchor the
    /// same way. Kept as its own test so each order-discriminating input
    /// reports independently — sharing one test would let the second assertion
    /// hide behind the first one's panic.
    #[test]
    fn a_split_parameter_name_is_still_redacted() {
        assert_eq!(
            rejected_href_for_log("https://evil.test/?to\rken=SECRET&p=3"),
            "https://evil.test/?token=***&p=3"
        );
    }

    #[test]
    fn test_next_listing_page_url_no_next() {
        let html = r#"<span class="pagi-btn-alt">5</span>"#;
        assert_eq!(next_listing_page_url(html, APEX_PAGE), None);
    }
}
