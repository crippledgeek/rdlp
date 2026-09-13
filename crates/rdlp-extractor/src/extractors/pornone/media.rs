//! Signed MP4 renditions from the server-rendered `<source>` tags.
//!
//! The rendered DOM is NOT trusted: in a browser `<video>.currentSrc` is a
//! pre-roll ad creative. The static HTML the server sends carries the real,
//! per-rendition signed URLs, and the site rejects every token substitution
//! (#659 measured 403/404 for all of them), so the served set is the format
//! list — one or several, never synthesised.

use lazy_regex::{Lazy, Regex, lazy_regex};
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::sync::LazyLock;

use crate::base::common::resolve_media_url;

static SOURCE_SELECTOR: LazyLock<Selector> = crate::static_selector!("video source[src]");

/// `…/<id>_<width>x<height>_<bitrate>k.mp4` — the filename is the only honest
/// statement of the rendition's geometry; `label`/`res` disagree with it.
static FILENAME_GEOMETRY: Lazy<Regex> =
    lazy_regex!(r"_(?P<w>\d+)x(?P<h>\d+)_(?P<kbps>\d+)k\.mp4(?:\?|\z)");

/// Host suffix every signed rendition is served from (`s1001.pornone.com`,
/// `s1004.pornone.com`, …); anything else under `<source>` is not a rendition.
const RENDITION_HOST_SUFFIX: &str = ".pornone.com";

pub(crate) struct Rendition {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub bitrate_kbps: u32,
}

/// Renditions are returned in DOM order — the order the page happens to list
/// `<source>` tags in, never sorted by height or bitrate. Callers must not
/// assume the first entry is the best (or worst) rendition.
pub(crate) fn parse_renditions(document: &Html, page_url: &str) -> Vec<Rendition> {
    let mut seen = HashSet::new();
    document
        .select(&SOURCE_SELECTOR)
        .filter_map(|source| {
            let src = source.value().attr("src")?;
            let url = resolve_media_url(page_url, src)?;
            let host = url::Url::parse(&url).ok()?.host_str()?.to_owned();
            if !host.ends_with(RENDITION_HOST_SUFFIX) {
                return None;
            }
            let geometry = FILENAME_GEOMETRY.captures(&url)?;
            let parse = |name: &str| geometry.name(name)?.as_str().parse::<u32>().ok();
            let rendition = Rendition {
                width: parse("w")?,
                height: parse("h")?,
                bitrate_kbps: parse("kbps")?,
                url,
            };
            seen.insert(rendition.url.clone()).then_some(rendition)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::Html;

    const VIDEO_PAGE: &str = include_str!("tests/pornone_video_page.html");
    const PAGE_URL: &str = "https://pornone.com/1080p/pornone-sex-video-278218023/278218023/";

    #[test]
    fn every_served_source_becomes_a_rendition_with_filename_dimensions() {
        let r = parse_renditions(&Html::parse_document(VIDEO_PAGE), PAGE_URL);
        assert!(!r.is_empty());
        for rend in &r {
            assert!(rend.url.starts_with("https://s"), "{}", rend.url);
            assert!(rend.url.contains(".pornone.com/vid"), "{}", rend.url);
            assert!(rend.width > 0 && rend.height > 0 && rend.bitrate_kbps > 0);
        }
        // The capture served a 720x406_500k file labelled "480p": dimensions
        // are read from the filename, never from label/res.
        let low = r
            .iter()
            .find(|x| x.url.contains("_720x406_500k"))
            .expect("480p-labelled source");
        assert_eq!((low.width, low.height, low.bitrate_kbps), (720, 406, 500));
    }

    #[test]
    fn label_and_res_attributes_are_ignored() {
        let html = r#"<video><source src="https://s1.pornone.com/vid2/sig/1/1/9/9_640x360_400k.mp4?lang=en" type="video/mp4" label="1080p" res="1080"/></video>"#;
        let r = parse_renditions(&Html::parse_document(html), PAGE_URL);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].width, r[0].height), (640, 360));
    }

    #[test]
    fn a_non_pornone_or_malformed_source_is_dropped() {
        // The ad-creative trap: a rendered player swaps in cdn.amplifo.com;
        // a server-rendered page never carries it as <source>, and if one
        // did, it is not a signed pornone rendition and must not be a format.
        let html = r#"<video>
            <source src="https://cdn.amplifo.com/data/creatives/x.mp4" type="video/mp4"/>
            <source src="javascript:alert(1)" type="video/mp4"/>
            <source src="" type="video/mp4"/>
            <source src="https://s1.pornone.com/vid2/sig/1/1/9/9_640x360_400k.mp4" type="video/mp4"/>
        </video>"#;
        let r = parse_renditions(&Html::parse_document(html), PAGE_URL);
        assert_eq!(r.len(), 1);
        assert!(r[0].url.ends_with("_640x360_400k.mp4"));
    }

    #[test]
    fn duplicate_sources_collapse() {
        let s = r#"<source src="https://s1.pornone.com/vid2/sig/1/1/9/9_640x360_400k.mp4" type="video/mp4"/>"#;
        let html = format!("<video>{s}{s}</video>");
        assert_eq!(
            parse_renditions(&Html::parse_document(&html), PAGE_URL).len(),
            1
        );
    }
}
