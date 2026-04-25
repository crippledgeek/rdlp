//! Live SpankBang extraction smoke test.
//!
//! Marked `#[ignore]` so it does not run in CI. Run manually with:
//!   cargo test -p rdlp-extractor --test spankbang_live -- --ignored --nocapture
//!
//! Requires network access and that the test URL still exists on the site.

use std::sync::Arc;

use rdlp_core::{ExtractionContext, InfoExtractor};
use rdlp_extractor::SpankBangExtractor;
use rdlp_http::HttpClientFactory;
use rdlp_jsinterp::BoaJsEngine;
use rdlp_types::Config;

const TEST_URL: &str = "https://spankbang.com/a4eph/video/dogfart+sexy+katalina+kyle+turns+a+messy+rental+inspection+into+wild+hardcore+sex";

#[tokio::test]
#[ignore]
async fn spankbang_live_extraction_yields_formats() {
    let http = Arc::new(HttpClientFactory::default().build());
    let js = Arc::new(BoaJsEngine::new());
    let cookies = Arc::new(rdlp_cookies::SimpleCookieJar::new());
    let cfg = Arc::new(Config::default());
    let ctx = ExtractionContext::new(http, js, cookies, cfg);

    let extractor = SpankBangExtractor::new();
    let info = extractor
        .extract(TEST_URL, &ctx)
        .await
        .expect("live extraction must succeed against the live URL");

    assert_eq!(info.extractor, "SpankBang");
    assert!(!info.formats.is_empty(), "must yield at least one format");
    assert!(
        info.formats.iter().any(|f| f.format_id == "1080p"),
        "1080p MP4 expected"
    );
    assert!(
        info.formats.iter().any(|f| f.format_id == "hls"),
        "HLS master expected"
    );
    assert!(info.title.to_lowercase().contains("dogfart"));
    assert_eq!(info.duration, Some(910.0));
}
