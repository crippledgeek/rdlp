//! HTML parser for PornHub's `/video/search` results page.
//!
//! Used as the **primary** path by `PornHubExtractor::search_page` — the
//! Webmaster JSON API does not carry uploader information, so the HTML
//! parse is what populates `SearchResultPreview.uploader`.

use lazy_regex::regex;
use rdlp_core::Result;
use rdlp_types::SearchResultPreview;
use scraper::{ElementRef, Html};

use crate::base::common::{resolve_card_url, resolve_media_url};

const SITE_BASE: &str = "https://www.pornhub.com";

/// Poster attributes in preference order; see [`parse_thumbnail`].
const THUMBNAIL_ATTRS: [&str; 3] = ["src", "data-mediumthumb", "data-image"];

/// Parse a PornHub HTML search-results page body.
///
/// Returns a vector of `SearchResultPreview`. An empty vector is a valid
/// result (zero matches on the page); only outright DOM-parse failure
/// returns `Err`.
pub(crate) fn parse_html_search_results(body: &str) -> Result<Vec<SearchResultPreview>> {
    let doc = Html::parse_document(body);
    let card_sel = crate::selector!("li.pcVideoListItem");

    let mut results = Vec::new();
    for card in doc.select(card_sel) {
        if let Some(preview) = parse_card(&card) {
            results.push(preview);
        }
    }

    Ok(results)
}

fn parse_card(card: &ElementRef<'_>) -> Option<SearchResultPreview> {
    let title_sel = crate::selector!("span.title a");
    let title_el = card.select(title_sel).next()?;
    let title = title_el.text().collect::<String>().trim().to_string();
    if title.is_empty() {
        return None;
    }

    let href = title_el.value().attr("href")?;
    if !href.contains("view_video.php?viewkey=") {
        return None;
    }
    // The `contains` above guards the PATH shape and says nothing about the
    // authority, so it left `href` free to choose the host (#665).
    let video_url = resolve_card_url(SITE_BASE, href)?;

    let (uploader, uploader_url) = parse_uploader(card);
    let duration = parse_duration(card);
    let view_count = parse_view_count(card);
    let thumbnail_url = parse_thumbnail(card);

    Some(SearchResultPreview {
        video_url,
        title,
        thumbnail_url,
        duration,
        uploader,
        uploader_url,
        actors: Vec::new(),
        view_count,
        upload_date: None,
    })
}

/// Returns `(display_name, absolute_url)` for the uploader anchor, if present.
///
/// One anchor element supplies both — the text node is the display name and the
/// `href` attribute is the channel/model/pornstar path — but they are not
/// returned together unconditionally: a named uploader whose `href` fails the
/// same-origin check comes back as `(Some(name), None)`. See
/// [`uploader_from_anchor`].
fn parse_uploader(card: &ElementRef<'_>) -> (Option<String>, Option<String>) {
    // Primary: badged link (~5% of cards observed live).
    let strict = crate::selector!(r#"div.usernameWrap span.usernameBadgesWrapper a"#);
    // Fallback: any link in the username wrapper (~95% coverage live).
    let loose = crate::selector!("div.usernameWrap a");
    for sel in [strict, loose] {
        if let Some((name, url)) = card.select(sel).next().and_then(uploader_from_anchor) {
            return (Some(name), url);
        }
    }
    (None, None)
}

/// `(display_name, absolute_url)` for one uploader anchor, or `None` when the
/// anchor carries no usable display name.
///
/// The inner `Option` is `None` in two cases the caller cannot tell apart and
/// treats alike: the anchor has no `href` at all, and — since #665 — it has one
/// whose resolved origin is not PornHub's. So an uploader can legitimately come
/// back named but unlinked; the name is still useful, and only the URL that
/// would have pointed off-site is dropped.
///
/// The two selector branches in [`parse_uploader`] were byte-identical here
/// before #665, which is how they both came to resolve the `href` by
/// concatenation; sharing the form means the authority check exists once.
fn uploader_from_anchor(el: ElementRef<'_>) -> Option<(String, Option<String>)> {
    let name = el.text().collect::<String>().trim().to_string();
    if name.is_empty() {
        return None;
    }
    let url = el
        .value()
        .attr("href")
        .and_then(|href| resolve_card_url(SITE_BASE, href));
    Some((name, url))
}

fn parse_duration(card: &ElementRef<'_>) -> Option<f64> {
    let sel = crate::selector!("var.duration");
    let text = card.select(sel).next()?.text().collect::<String>();
    let trimmed = text.trim();
    let mut secs = 0_u64;
    for part in trimmed.split(':') {
        let n: u64 = part.parse().ok()?;
        secs = secs * 60 + n;
    }
    Some(secs as f64)
}

fn parse_view_count(card: &ElementRef<'_>) -> Option<u64> {
    let sel = crate::selector!(".views var");
    let text = card.select(sel).next()?.text().collect::<String>();
    parse_view_count_text(&text)
}

/// Parse "844K", "844K views", "3.7M", "1.2B".
pub(crate) fn parse_view_count_text(s: &str) -> Option<u64> {
    let re = regex!(r"(?i)([0-9]+(?:\.[0-9]+)?)\s*([KMB]?)");
    let caps = re.captures(s)?;
    let n: f64 = caps.get(1)?.as_str().parse().ok()?;
    let mult: f64 = match caps.get(2)?.as_str().to_ascii_uppercase().as_str() {
        "K" => 1_000.0,
        "M" => 1_000_000.0,
        "B" => 1_000_000_000.0,
        _ => 1.0,
    };
    let total = (n * mult).round();
    if total < 0.0 {
        None
    } else {
        Some(total as u64)
    }
}

/// The poster for one card, resolved and restricted to `http(s)`.
///
/// [`resolve_media_url`], not [`resolve_card_url`]: a poster legitimately lives
/// on a different host from the listing (PornHub's are on `*.phncdn.com`), so
/// an origin comparison would drop every real thumbnail. What it does refuse is
/// a non-`http(s)` reference — a `data:` or `javascript:` value reaching the
/// desktop's `<img src>` — and a relative `src` handed to the UI unresolved.
/// Same treatment eporner and pornoxo already give their posters.
///
/// Resolution is tried PER candidate, not once after the fallback chain has
/// committed: on a lazy-loaded card `src` is a `data:` placeholder and the real
/// poster is in `data-mediumthumb`, so resolving the winner would return `None`
/// and leave the remaining attributes unreachable. eporner measured that shape
/// at 81% of its results.
fn parse_thumbnail(card: &ElementRef<'_>) -> Option<String> {
    let sel = crate::selector!("img");
    let img = card.select(sel).next()?;
    THUMBNAIL_ATTRS
        .iter()
        .filter_map(|attr| img.value().attr(attr))
        // An empty attribute joins to the BASE, so it would resolve
        // "successfully" to the site root rather than being skipped.
        .filter(|src| !src.is_empty())
        .find_map(|src| resolve_media_url(SITE_BASE, src))
}

#[cfg(test)]
mod unit_tests {
    use super::{SITE_BASE, parse_html_search_results, parse_view_count_text};

    /// An absolute or protocol-relative href that the site's own path-shape
    /// guard (`contains("view_video.php?viewkey=")`) says nothing about.
    /// Before #665 the first one was passed through verbatim by the
    /// `starts_with("http")` arm.
    const AUTHORITY_CARRYING: [&str; 2] = [
        "https://evil.test/view_video.php?viewkey=ph000",
        "//evil.test/view_video.php?viewkey=ph000",
    ];

    /// References that LOOK like an authority and are not: RFC 3986 resolution
    /// turns both into on-origin paths, so a correct fix keeps the card.
    const NEUTRALISED: [&str; 2] = [
        "@evil.test/view_video.php?viewkey=ph000",
        ".evil.test/view_video.php?viewkey=ph000",
    ];

    const LEGIT_HREF: &str = "/view_video.php?viewkey=ph111";
    const LEGIT_UPLOADER_HREF: &str = "/model/real-uploader";

    fn host_of(url: &str) -> String {
        url::Url::parse(url)
            .expect("every emitted URL must parse")
            .host_str()
            .expect("every emitted URL must carry a host")
            .to_string()
    }

    /// One `li.pcVideoListItem` with the loose (unbadged) uploader shape.
    fn card(href: &str, title: &str, uploader_href: &str) -> String {
        format!(
            r#"<li class="pcVideoListItem">
                 <span class="title"><a href="{href}">{title}</a></span>
                 <div class="usernameWrap"><a href="{uploader_href}">Uploader</a></div>
               </li>"#
        )
    }

    /// The same card with the badged (strict-selector) uploader shape.
    fn badged_card(href: &str, title: &str, uploader_href: &str) -> String {
        format!(
            r#"<li class="pcVideoListItem">
                 <span class="title"><a href="{href}">{title}</a></span>
                 <div class="usernameWrap"><span class="usernameBadgesWrapper">
                   <a href="{uploader_href}">Badged</a>
                 </span></div>
               </li>"#
        )
    }

    fn page(cards: &str) -> String {
        format!("<html><body><ul>{cards}</ul></body></html>")
    }

    fn parse(html: &str) -> Vec<rdlp_types::SearchResultPreview> {
        parse_html_search_results(html).expect("well-formed page must parse")
    }

    #[test]
    fn no_result_can_move_the_authority_off_pornhub() {
        for hostile in AUTHORITY_CARRYING {
            let html = page(&format!(
                "{}{}",
                card(hostile, "Hostile", LEGIT_UPLOADER_HREF),
                card(LEGIT_HREF, "Real", LEGIT_UPLOADER_HREF)
            ));
            let results = parse(&html);
            // Asserting the count as well as the host keeps this honest: a
            // resolution regression that emptied the page would otherwise read
            // as a green guard.
            assert_eq!(
                results.len(),
                1,
                "href {hostile:?}: the hostile card must be dropped and the legitimate one kept"
            );
            assert_eq!(results[0].title, "Real");
            assert_eq!(host_of(&results[0].video_url), "www.pornhub.com");
        }
    }

    #[test]
    fn no_uploader_url_can_move_the_authority_off_pornhub() {
        for hostile in AUTHORITY_CARRYING {
            for build in [card, badged_card] {
                let html = page(&build(LEGIT_HREF, "Real", hostile));
                let results = parse(&html);
                assert_eq!(results.len(), 1, "the card itself must survive");
                // The name is still usable; only the off-origin URL is dropped.
                assert!(results[0].uploader.is_some());
                assert_eq!(
                    results[0].uploader_url, None,
                    "uploader href {hostile:?} must be dropped, got {:?}",
                    results[0].uploader_url
                );
            }
        }
    }

    #[test]
    fn userinfo_and_lookalike_hrefs_stay_on_origin() {
        for href in NEUTRALISED {
            let html = page(&card(href, "Neutralised", href));
            let results = parse(&html);
            assert_eq!(
                results.len(),
                1,
                "href {href:?} resolves on-origin and must be kept"
            );
            assert_eq!(host_of(&results[0].video_url), "www.pornhub.com");
            assert_eq!(
                results[0].video_url,
                format!("{SITE_BASE}/{href}"),
                "href {href:?} must resolve to an on-origin PATH"
            );
            let uploader_url = results[0]
                .uploader_url
                .as_deref()
                .expect("an on-origin uploader href must be kept");
            assert_eq!(host_of(uploader_url), "www.pornhub.com");
        }
    }

    #[test]
    fn ordinary_root_relative_hrefs_resolve() {
        let html = page(&card(LEGIT_HREF, "Real", LEGIT_UPLOADER_HREF));
        let results = parse(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].video_url,
            format!("{SITE_BASE}{LEGIT_HREF}"),
            "a normal root-relative card href must still yield a usable URL"
        );
        assert_eq!(
            results[0].uploader_url.as_deref(),
            Some(format!("{SITE_BASE}{LEGIT_UPLOADER_HREF}").as_str())
        );
    }

    /// Posters go through `resolve_media_url`, so an off-origin CDN host is
    /// kept (PornHub's real thumbs are on `*.phncdn.com`), a relative one is
    /// absolutized rather than handed to the UI as `/t.jpg`, and a `data:`
    /// reference never reaches the desktop's `<img src>`. A bad poster costs
    /// the poster, not the card.
    #[test]
    fn posters_are_resolved_and_non_http_ones_dropped() {
        let card_with_img = |src: &str| {
            format!(
                r#"<li class="pcVideoListItem">
                     <span class="title"><a href="{LEGIT_HREF}">Real</a></span>
                     <img src="{src}">
                   </li>"#
            )
        };
        let cdn = "https://ei.phncdn.com/videos/real.jpg";
        let html = page(&format!(
            "{}{}{}",
            card_with_img(cdn),
            card_with_img("/t.jpg"),
            card_with_img("data:text/html,x")
        ));
        let results = parse(&html);
        assert_eq!(results.len(), 3, "a bad poster must not cost the card");
        assert_eq!(results[0].thumbnail_url.as_deref(), Some(cdn));
        assert_eq!(
            results[1].thumbnail_url.as_deref(),
            Some(format!("{SITE_BASE}/t.jpg").as_str())
        );
        assert_eq!(results[2].thumbnail_url, None);
    }

    /// A `data:` placeholder in `src` must fall through to `data-mediumthumb`,
    /// not consume the card's only chance at a poster. Resolving once after the
    /// `or_else` chain had committed would return `None` here — eporner
    /// measured this lazy shape at 81% of its results.
    #[test]
    fn a_placeholder_src_falls_through_to_the_lazy_attribute() {
        let real = "https://ei.phncdn.com/videos/real.jpg";
        // Both lazy attributes, so the walk is exercised to its second AND
        // third candidate — a `find_map` that stopped after one fallback would
        // pass the first case and fail the second.
        for lazy_attr in ["data-mediumthumb", "data-image"] {
            let html = page(&format!(
                r#"<li class="pcVideoListItem">
                     <span class="title"><a href="{LEGIT_HREF}">Real</a></span>
                     <img src="data:image/gif;base64,R0lGOD" {lazy_attr}="{real}">
                   </li>"#
            ));
            let results = parse(&html);
            assert_eq!(results.len(), 1);
            assert_eq!(
                results[0].thumbnail_url.as_deref(),
                Some(real),
                "a data: src must fall through to {lazy_attr}"
            );
        }
    }

    /// The badged (strict) selector is a separate branch from the loose one and
    /// resolves its href the same way.
    #[test]
    fn badged_uploader_href_resolves() {
        let html = page(&badged_card(LEGIT_HREF, "Real", LEGIT_UPLOADER_HREF));
        let results = parse(&html);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].uploader.as_deref(), Some("Badged"));
        assert_eq!(
            results[0].uploader_url.as_deref(),
            Some(format!("{SITE_BASE}{LEGIT_UPLOADER_HREF}").as_str())
        );
    }

    #[test]
    fn parse_views_k_suffix() {
        assert_eq!(parse_view_count_text("844K views"), Some(844_000));
    }

    #[test]
    fn parse_views_m_suffix() {
        assert_eq!(parse_view_count_text("3.7M"), Some(3_700_000));
    }

    #[test]
    fn parse_views_no_suffix() {
        assert_eq!(parse_view_count_text("12345"), Some(12_345));
    }

    #[test]
    fn parse_views_garbage_returns_none() {
        assert_eq!(parse_view_count_text("???"), None);
    }
}
