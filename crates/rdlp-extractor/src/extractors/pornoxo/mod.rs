//! PornoXO extractor.
//!
//! The video page embeds a signed HLS ladder in an inline `playerConfig`
//! block. The signature is minted per page load and expires, so the master
//! URL is fetched and expanded during extraction and never cached.

mod patterns;
mod player;

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
use crate::hls::detect_format_sizes_lazy;

/// PornoXO — signed per-page-load HLS ladder read from an inline `playerConfig`.
pub struct PornoxoExtractor;

impl PornoxoExtractor {
    /// Create a new PornoXO extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for PornoxoExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for PornoxoExtractor {
    fn name(&self) -> &str {
        "PornoXO"
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
            .ok_or_else(|| RdlpError::extraction("URL is not a PornoXO video page", url))?;

        debug!(
            "[PornoXO] Extracting {video_id} from {}",
            rdlp_redact::RedactedUrl::new(url)
        );

        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        let master_url = player::extract_master_url(&webpage)
            .map_err(|e| RdlpError::extraction(format!("{e:#}"), url))?;

        // The master URL comes from page JavaScript and is therefore
        // attacker-controlled. `expand_hls_url` does not validate its own seed
        // URL, so gate it here before any fetch. Shares the playlist-URI gate
        // rather than restating it, so production behaviour cannot drift
        // between the master and the variants it resolves.
        crate::hls::validate_resolved_url(&master_url)
            .map_err(|e| RdlpError::extraction(format!("master URL rejected: {e}"), url))?;

        // `Html` is !Send, so parse and drop it before any await.
        let json_ld = extract_json_ld(&Html::parse_document(&webpage));

        // Explicit M3u8: the master URL ends in `.mp4`, so extension sniffing
        // would misclassify it as progressive HTTP.
        let seed = Format::new("hls", &master_url, "mp4", DownloadProtocol::M3u8);

        // MUST run before `detect_format_sizes_lazy` (issue #269 / #279).
        let formats = crate::hls::expand_hls_in_place(vec![seed], ctx.http_client.clone()).await;
        let extractor_name = InfoExtractor::name(self);
        let (formats, hls_flags) = detect_format_sizes_lazy(formats, ctx, extractor_name).await;

        if formats.is_empty() {
            return Err(RdlpError::extraction(
                "signed HLS master expanded to no formats",
                url,
            ));
        }

        let title = json_ld
            .as_ref()
            .and_then(|j| j.name.clone())
            .unwrap_or_else(|| video_id.clone());

        let mut info = InfoDict::new(&video_id, &title, extractor_name, url);
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
        if hls_flags.is_live {
            info.is_live = Some(true);
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hls::test_support::*;

    const VIDEO_PAGE: &str = include_str!("tests/pornoxo_video_page.html");

    #[test]
    fn suitable_matches_only_video_urls() {
        let x = PornoxoExtractor::new();
        assert!(InfoExtractor::suitable(
            &x,
            "https://www.pornoxo.com/videos/2928541/slug/"
        ));
        assert!(!InfoExtractor::suitable(
            &x,
            "https://www.pornoxo.com/tags/creampie/"
        ));
    }

    #[tokio::test]
    async fn extracts_three_formats_and_metadata_from_signed_ladder() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        // Master playlist: three variants, absolute-path URIs (as the live site serves).
        let master = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1201052,RESOLUTION=854x480,CODECS=\"mp4a.40.2,avc1.4d4015\"\n\
            /v/480.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=706527,RESOLUTION=426x240,CODECS=\"mp4a.40.2,avc1.4d4015\"\n\
            /v/240.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2835133,RESOLUTION=1280x720,CODECS=\"mp4a.40.2,avc1.4d4015\"\n\
            /v/720.m3u8\n";
        let media = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nseg0.ts\n#EXT-X-ENDLIST\n";

        // Rewrite the fixture's CDN host to the mock server so the whole
        // page -> master -> variant chain is exercised end to end.
        let page = VIDEO_PAGE.replace("https:\\/\\/cdn.pornoxo.com", &base.replace('/', "\\/"));

        let _page_mock = server
            .mock("GET", "/videos/2928541/x/")
            .with_status(200)
            .with_body(&page)
            .create_async()
            .await;
        let _master_mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/key=.*_TPL_\.mp4$".into()),
            )
            .with_status(200)
            .with_body(master)
            .create_async()
            .await;
        let _v480 = server
            .mock("GET", "/v/480.m3u8")
            .with_status(200)
            .with_body(media)
            .create_async()
            .await;
        let _v240 = server
            .mock("GET", "/v/240.m3u8")
            .with_status(200)
            .with_body(media)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v/720.m3u8")
            .with_status(200)
            .with_body(media)
            .create_async()
            .await;

        let ctx = test_ctx();
        let info = PornoxoExtractor::new()
            .extract(&format!("{base}/videos/2928541/x/"), &ctx)
            .await
            .expect("extraction should succeed");

        assert_eq!(info.id, "2928541");
        assert_eq!(
            info.title,
            "He Fucked his Stepsister while she was on the Phone"
        );
        assert_eq!(info.duration, Some(930.0));
        assert_eq!(info.age_limit, Some(18));
        assert_eq!(info.formats.len(), 3, "one Format per signed rendition");
        let mut heights: Vec<_> = info.formats.iter().filter_map(|f| f.height).collect();
        heights.sort_unstable();
        assert_eq!(heights, vec![240, 480, 720]);
        assert!(
            info.formats
                .iter()
                .all(|f| f.protocol == DownloadProtocol::M3u8)
        );
        assert!(
            info.tags
                .as_ref()
                .is_some_and(|t| t.iter().any(|k| k == "Creampie"))
        );
    }

    #[tokio::test]
    async fn rejects_a_master_url_that_fails_security_validation() {
        // A page whose playerConfig points at a private host must be refused
        // before any fetch — the URL is attacker-controlled page JavaScript.
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        let page = r#"<script>var playerConfig = { sources: {"hlsAuto":"http://169.254.169.254/latest/meta-data/"}, };</script>"#;
        let _m = server
            .mock("GET", "/videos/1/x/")
            .with_status(200)
            .with_body(page)
            .create_async()
            .await;

        let err = PornoxoExtractor::new()
            .extract(&format!("{base}/videos/1/x/"), &test_ctx())
            .await
            .expect_err("link-local master URL must be refused");
        assert!(matches!(err, RdlpError::Extraction { .. }));
        // Pin the REASON, not just that something failed: without this the test
        // would pass on any unrelated error (a failed page fetch, a parse slip)
        // and stop guarding the SSRF gate at all.
        assert!(
            err.to_string().contains("master URL rejected"),
            "must fail at the security gate, got: {err}"
        );
    }

    #[tokio::test]
    async fn errors_when_page_has_no_player_config() {
        let mut server = mockito::Server::new_async().await;
        let base = server.url();
        let _m = server
            .mock("GET", "/videos/1/x/")
            .with_status(200)
            .with_body("<html><body>gone</body></html>")
            .create_async()
            .await;

        let err = PornoxoExtractor::new()
            .extract(&format!("{base}/videos/1/x/"), &test_ctx())
            .await
            .expect_err("a page with no playerConfig must error");
        assert!(err.to_string().contains("playerConfig"), "got: {err}");
    }
}
