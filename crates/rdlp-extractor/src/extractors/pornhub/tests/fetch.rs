//! Mockito-based integration tests for `PornHubExtractor::fetch_html_search_page`
//! and `fetch_api_search_page`.
//!
//! These tests verify the HTTP-fetch + parse pipeline in isolation. The fallback
//! ORDER between the two helpers is validated by code review of the PR #231 diff
//! (the URL builder hardcodes `https://www.pornhub.com` so the switchover logic
//! cannot be redirected to mockito without invasive refactoring).

use std::sync::Arc;

use async_trait::async_trait;
use mockito::Server;
use rdlp_core::{CookieJar, ExtractionContext, JsEngine, Result};
use rdlp_types::Config;

use crate::extractors::pornhub::PornHubExtractor;

// ---- Test support --------------------------------------------------------

struct NoOpJsEngine;

#[async_trait]
impl JsEngine for NoOpJsEngine {
    async fn eval(&self, _code: &str) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
    async fn eval_with_context(
        &self,
        _code: &str,
        _context: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
    async fn call_function(
        &self,
        _name: &str,
        _args: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

struct NoOpCookieJar;

#[async_trait]
impl CookieJar for NoOpCookieJar {
    async fn get_cookies(&self, _url: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn add_cookie(&self, _url: &str, _cookie: &str) -> Result<()> {
        Ok(())
    }
    async fn load_from_browser(&self, _browser: rdlp_types::BrowserType) -> Result<usize> {
        Ok(0)
    }
    async fn load_from_file(&self, _path: &std::path::Path) -> Result<usize> {
        Ok(0)
    }
}

fn test_ctx(_server: &Server) -> ExtractionContext {
    let client = wreq::Client::builder()
        .redirect(wreq::redirect::Policy::none())
        .build()
        .expect("client build must succeed in tests");
    let config = Config {
        verbose: false,
        ..Config::default()
    };
    ExtractionContext::new(
        Arc::new(client),
        Arc::new(NoOpJsEngine),
        Arc::new(NoOpCookieJar),
        Arc::new(config),
    )
}

/// Build a minimal PornHub-style search-results HTML body.
///
/// Each `(title, uploader, uploader_href, duration_secs)` tuple generates one
/// `li.pcVideoListItem` card with the fields filled in.
fn ph_search_html(
    cards: &[(&str, &str, &str, &str, &str)], // (viewkey, title, uploader, uploader_href, duration)
) -> String {
    let items: String = cards
        .iter()
        .map(|(key, title, uploader, href, duration)| {
            format!(
                r#"<li class="pcVideoListItem">
                    <div class="wrap">
                        <span class="title">
                            <a href="/view_video.php?viewkey={key}">{title}</a>
                        </span>
                        <div class="usernameWrap">
                            <a href="{href}">{uploader}</a>
                        </div>
                        <var class="duration">{duration}</var>
                        <div class="views"><var>1.2M views</var></div>
                        <img src="https://ci.phncdn.com/{key}.jpg" />
                    </div>
                </li>"#
            )
        })
        .collect();
    format!("<html><body><ul>{items}</ul></body></html>")
}

/// Build a minimal PornHub Webmaster API JSON response.
fn ph_api_json(videos: &[(&str, &str, &str, &str)]) -> String {
    // (video_id, title, thumb, duration)
    let video_objs: String = videos
        .iter()
        .map(|(vid_id, title, thumb, duration)| {
            format!(
                r#"{{
                    "video_id": "{vid_id}",
                    "title": "{title}",
                    "url": "https://www.pornhub.com/view_video.php?viewkey={vid_id}",
                    "thumb": "{thumb}",
                    "duration": "{duration}",
                    "views": 42,
                    "tags": [],
                    "pornstars": [],
                    "categories": []
                }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"videos": [{video_objs}]}}"#)
}

// ---- fetch_html_search_page tests ----------------------------------------

#[tokio::test]
async fn fetch_html_search_page_parses_against_mockito() {
    let mut server = Server::new_async().await;

    let html = ph_search_html(&[
        (
            "ph111aaa",
            "Title Alpha",
            "UploaderOne",
            "/model/uploaderone",
            "10:00",
        ),
        (
            "ph222bbb",
            "Title Beta",
            "UploaderTwo",
            "/channels/uploadertwo",
            "5:30",
        ),
        (
            "ph333ccc",
            "Title Gamma",
            "UploaderThree",
            "/pornstar/uploaderthree",
            "20:15",
        ),
    ]);
    let mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/html; charset=utf-8")
        .with_body(&html)
        .create_async()
        .await;

    let extractor = PornHubExtractor::new();
    let ctx = test_ctx(&server);
    let url = format!("{}/video/search?search=test&page=1", server.url());

    let results = extractor
        .fetch_html_search_page(&url, &ctx)
        .await
        .expect("fetch_html_search_page must succeed against a 200 response");

    assert_eq!(results.len(), 3, "expected 3 parsed cards");
    assert_eq!(results[0].title, "Title Alpha");
    assert_eq!(results[1].title, "Title Beta");
    assert_eq!(results[2].title, "Title Gamma");

    assert_eq!(
        results[0].uploader.as_deref(),
        Some("UploaderOne"),
        "uploader name must be populated"
    );
    assert_eq!(
        results[0].uploader_url.as_deref(),
        Some("https://www.pornhub.com/model/uploaderone"),
        "uploader_url must be resolved to absolute"
    );
    assert_eq!(results[0].duration, Some(600.0), "10:00 = 600s");

    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_html_search_page_propagates_5xx_as_error() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(503)
        .with_body("Service Unavailable")
        .create_async()
        .await;

    let extractor = PornHubExtractor::new();
    let ctx = test_ctx(&server);
    let url = format!("{}/video/search?search=test&page=1", server.url());

    let result = extractor.fetch_html_search_page(&url, &ctx).await;
    assert!(
        result.is_err(),
        "a 503 response must propagate as Err, got: {result:?}"
    );

    mock.assert_async().await;
}

// ---- fetch_api_search_page tests -----------------------------------------

#[tokio::test]
async fn fetch_api_search_page_parses_against_mockito() {
    let mut server = Server::new_async().await;

    let json = ph_api_json(&[(
        "ph777xxx",
        "API Video One",
        "https://ci.phncdn.com/thumb.jpg",
        "5:00",
    )]);
    let mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&json)
        .create_async()
        .await;

    let extractor = PornHubExtractor::new();
    let ctx = test_ctx(&server);
    let url = format!(
        "{}/webmasters/search?search=test&page=1&ordering=newest",
        server.url()
    );

    let results = extractor
        .fetch_api_search_page(&url, &ctx)
        .await
        .expect("fetch_api_search_page must succeed against a 200 response");

    assert_eq!(results.len(), 1, "expected 1 parsed video");
    assert_eq!(results[0].title, "API Video One");
    assert_eq!(results[0].duration, Some(300.0), "5:00 = 300s");
    assert!(
        results[0].uploader.is_none(),
        "API parser never populates uploader (Webmaster API does not expose it)"
    );
    assert!(
        results[0].uploader_url.is_none(),
        "API parser never populates uploader_url"
    );

    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_api_search_page_propagates_5xx_as_error() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(503)
        .with_body("Service Unavailable")
        .create_async()
        .await;

    let extractor = PornHubExtractor::new();
    let ctx = test_ctx(&server);
    let url = format!("{}/webmasters/search?search=test&page=1", server.url());

    let result = extractor.fetch_api_search_page(&url, &ctx).await;
    assert!(
        result.is_err(),
        "a 503 response must propagate as Err, got: {result:?}"
    );

    mock.assert_async().await;
}
