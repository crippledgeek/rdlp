//! pornone.com extractor.
//!
//! The video page serves the real, per-rendition signed MP4 URLs directly in
//! static HTML (see `media.rs`) — there is no separate manifest or size
//! endpoint to probe, and the signed URLs are short-lived, so `extract` never
//! runs `detect_format_sizes_lazy`/HEAD probing: the served geometry and
//! bitrate are all the information the site offers, and a probe would only
//! spend one of the URL's few remaining minutes for no gain.

mod media;
mod patterns;

use async_trait::async_trait;
use lazy_regex::Regex;
use log::debug;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::{DownloadProtocol, Format, InfoDict};
use scraper::Html;

use crate::base::common::BaseExtractor;
use crate::base::common::json_ld::{
    extract_json_ld, extract_tags, extract_view_count, get_thumbnail_url,
};

/// pornone.com — server-rendered, individually-signed progressive MP4 renditions.
///
/// A unit struct here, not yet a `SearchOrigin`-carrying one: no video
/// extraction path needs a listing origin, and Task 4 (`search.rs`) is what
/// introduces the field, its constructor plumbing, and the `#[cfg(test)]
/// with_origin` seam together with their first real callers — adding them a
/// task early would leave both dead until then.
pub struct PornoneExtractor;

impl PornoneExtractor {
    /// Create a new PornOne extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PornoneExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for PornoneExtractor {
    fn name(&self) -> &str {
        "PornOne"
    }

    fn valid_url(&self) -> &Regex {
        &patterns::URL_PATTERN
    }

    fn priority(&self) -> i32 {
        50
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let video_id = patterns::parse_video_id(url)
            .ok_or_else(|| RdlpError::extraction("URL is not a PornOne video page", url))?;

        debug!(
            "[PornOne] Extracting {video_id} from {}",
            rdlp_redact::RedactedUrl::new(url)
        );

        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // `Html` is !Send — parse, take what we need, drop before any await.
        let (renditions, json_ld) = {
            let document = Html::parse_document(&webpage);
            (
                media::parse_renditions(&document, url),
                extract_json_ld(&document),
            )
        };
        if renditions.is_empty() {
            return Err(RdlpError::extraction(
                "page served no signed <source> rendition",
                url,
            ));
        }

        let mut formats = Vec::with_capacity(renditions.len());
        for r in renditions {
            // Page-sourced URL: gate before it can be probed or returned.
            BaseExtractor::validate_url_security(&r.url)?;
            let mut f = Format::new(
                format!("mp4-{}p", r.height),
                &r.url,
                "mp4",
                DownloadProtocol::Https,
            );
            f.width = Some(r.width);
            f.height = Some(r.height);
            f.tbr = Some(f64::from(r.bitrate_kbps));
            formats.push(f);
        }

        let title = json_ld
            .as_ref()
            .and_then(|j| j.name.clone())
            .unwrap_or_else(|| video_id.clone());
        let mut info = InfoDict::new(&video_id, &title, InfoExtractor::name(self), url);
        info.formats = formats;
        info.age_limit = Some(18);
        if let Some(j) = &json_ld {
            info.description = j.description.clone();
            info.thumbnail = get_thumbnail_url(j);
            info.upload_date = j
                .upload_date
                .as_deref()
                .and_then(BaseExtractor::parse_iso8601_date);
            info.duration = j
                .duration
                .as_deref()
                .and_then(BaseExtractor::parse_iso8601_duration);
            info.tags = extract_tags(j);
            info.view_count = extract_view_count(j);
        }
        info.propagate_duration();
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hls::test_support::*;

    const VIDEO_PAGE: &str = include_str!("tests/pornone_video_page.html");

    #[test]
    fn name_priority_and_routing() {
        let e = PornoneExtractor::new();
        assert_eq!(InfoExtractor::name(&e), "PornOne");
        assert_eq!(e.priority(), 50);
        assert!(e.suitable("https://pornone.com/ass/slug/42/"));
        assert!(!e.suitable("https://pornone.com/search/?q=x"));
    }

    #[tokio::test]
    async fn extract_yields_one_format_per_served_source_with_json_ld_metadata() {
        // mockito serves the captured page; formats are the fixture's signed
        // URLs (real pornone hosts, so the SSRF gate admits them).
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/1080p/pornone-sex-video-278218023/278218023/")
            .with_body(VIDEO_PAGE)
            .create_async()
            .await;
        let ctx = test_ctx();
        let url = format!(
            "{}/1080p/pornone-sex-video-278218023/278218023/",
            server.url()
        );
        // `is_suitable` is anchored on pornone.com, so call `extract` directly.
        let info = PornoneExtractor::new()
            .extract(&url, &ctx)
            .await
            .expect("extract");
        assert_eq!(info.id, "278218023");
        assert_eq!(info.title, "milf");
        assert_eq!(info.duration, Some(139.0));
        assert_eq!(info.age_limit, Some(18));
        assert!(
            info.thumbnail
                .as_deref()
                .is_some_and(|t| t.starts_with("https://th-eu4.pornone.com/"))
        );
        assert!(!info.formats.is_empty());
        for f in &info.formats {
            assert_eq!(f.ext, "mp4");
            assert_eq!(f.protocol, DownloadProtocol::Https);
            assert!(f.width.is_some() && f.height.is_some());
            assert_eq!(f.duration, Some(139.0), "propagate_duration");
        }
        let ids: Vec<&str> = info.formats.iter().map(|f| f.format_id.as_str()).collect();
        assert!(ids.contains(&"mp4-406p"), "{ids:?}");
    }

    #[tokio::test]
    async fn extract_fails_clearly_when_no_source_is_served() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/ass/slug/1/")
            .with_body("<html><body>no player</body></html>")
            .create_async()
            .await;
        let ctx = test_ctx();
        let err = PornoneExtractor::new()
            .extract(&format!("{}/ass/slug/1/", server.url()), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no signed <source>"), "{err}");
    }
}
