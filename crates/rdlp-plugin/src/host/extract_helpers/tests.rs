use super::*;
use crate::bindings::rdlp::plugin::host_extract_helpers::{Host as _, RegexFlags};

fn ctx() -> PluginStoreData {
    PluginStoreData::new("test", tokio_util::sync::CancellationToken::new())
}

#[test]
fn search_regex_finds_first_match() {
    let mut c = ctx();
    let r = c.search_regex(
        r"(\d+)".to_string(),
        "id=42 ts=99".to_string(),
        RegexFlags::empty(),
    );
    assert_eq!(r, Some("42".to_string()));
}

/// Regression guard for L2: `_search_regex` must return group 1 even when
/// it captures an empty string (i.e. when `group(1)` exists but is empty).
///
/// Before the fix the loop skipped `None`/empty groups and fell through to
/// group 0, diverging from yt-dlp's `m.group(1)` unconditional return.
///
/// Pattern `(a)(b)?` against `"a"`:
///   - group 1 = "a", group 2 = None (optional, didn't match)
///   - expected: "a" (group 1)
#[test]
fn search_regex_group1_wins_over_group0_regression() {
    let mut c = ctx();
    let r = c.search_regex(r"(a)(b)?".to_string(), "a".to_string(), RegexFlags::empty());
    assert_eq!(r, Some("a".to_string()), "group 1 must be returned");
}

/// Regression guard for L2 (empty capture): pattern `(x)?(y)` against `"y"`:
///   - group 1 did not participate (optional `x` not present) → captured empty/None
///   - group 2 = "y"
///   - yt-dlp returns `m.group(1)` which is `""` (empty string)
///   - before the fix: old loop skipped the non-participating group and returned "y" (group 2)
///
/// Note: in the `regex` crate, a non-participating optional group returns
/// `None` from `m.get(1)`. Our implementation maps that to `""` to match
/// yt-dlp's `m.group(1)` returning `""` for a non-participating group in Python's re.
#[test]
fn search_regex_returns_empty_string_for_non_participating_group1() {
    let mut c = ctx();
    let r = c.search_regex(r"(x)?(y)".to_string(), "y".to_string(), RegexFlags::empty());
    // group 1 did not participate → empty string (not "y" from group 2)
    assert_eq!(
        r,
        Some(String::new()),
        "non-participating group 1 must yield empty string, not group 2's value"
    );
}

/// Sanity check: no capture groups → return the whole match (group 0).
#[test]
fn search_regex_no_groups_returns_whole_match() {
    let mut c = ctx();
    let r = c.search_regex(
        r"\d+".to_string(),
        "abc 42 def".to_string(),
        RegexFlags::empty(),
    );
    assert_eq!(r, Some("42".to_string()));
}

#[test]
fn search_regex_no_match_returns_none() {
    let mut c = ctx();
    let r = c.search_regex(
        r"NOPE".to_string(),
        "irrelevant".to_string(),
        RegexFlags::empty(),
    );
    assert_eq!(r, None);
}

#[test]
fn search_regex_ignore_case_flag() {
    let mut c = ctx();
    let r = c.search_regex(
        r"(foo)".to_string(),
        "FOO bar".to_string(),
        RegexFlags::IGNORE_CASE,
    );
    assert_eq!(r, Some("FOO".to_string()));
}

#[test]
fn html_search_regex_strips_tags() {
    let mut c = ctx();
    let r = c.html_search_regex(
        r"<title>(.+?)</title>".to_string(),
        "<title>Hello <b>World</b></title>".to_string(),
        RegexFlags::empty(),
    );
    assert_eq!(r, Some("Hello World".to_string()));
}

#[test]
fn html_search_regex_returns_entities_verbatim() {
    let mut c = ctx();
    let r = c.html_search_regex(
        r"<title>(.+?)</title>".to_string(),
        "<title>Tom &amp; Jerry</title>".to_string(),
        RegexFlags::empty(),
    );
    // Returned VERBATIM. The shim used to unescape here, matching yt-dlp;
    // it no longer does, because `InfoDict::decode_text_fields` decodes
    // whatever a plugin returns and two single-pass decodes compose into the
    // double-decode the boundary exists to prevent (#698).
    assert_eq!(r, Some("Tom &amp; Jerry".to_string()));
}

#[test]
fn html_search_meta_property_form() {
    let mut c = ctx();
    let r = c.html_search_meta(
        "og:title".to_string(),
        r#"<meta property="og:title" content="Hello"/>"#.to_string(),
    );
    assert_eq!(r, Some("Hello".to_string()));
}

#[test]
fn html_search_meta_name_form() {
    let mut c = ctx();
    let r = c.html_search_meta(
        "description".to_string(),
        r#"<meta name="description" content="Page text"/>"#.to_string(),
    );
    assert_eq!(r, Some("Page text".to_string()));
}

#[test]
fn html_search_meta_missing_returns_none() {
    let mut c = ctx();
    let r = c.html_search_meta("nope".to_string(), "<html></html>".to_string());
    assert_eq!(r, None);
}

/// Regression guard for L3: `_html_search_meta` must not truncate content
/// at a single-quote when the content is enclosed in double quotes.
///
/// Before the fix `[^"']*` stopped at the first `'` inside `"a'b"`,
/// returning `"a"` instead of `"a'b"`.
#[test]
fn html_search_meta_mixed_quote_content_not_truncated_regression() {
    let mut c = ctx();
    // content is double-quoted but contains an apostrophe
    let r = c.html_search_meta(
        "x".to_string(),
        r#"<meta name="x" content="a'b">"#.to_string(),
    );
    assert_eq!(
        r,
        Some("a'b".to_string()),
        "content containing single-quote inside double-quoted attribute must not be truncated"
    );
}

/// Companion: content in single quotes may contain a double-quote.
#[test]
fn html_search_meta_double_quote_inside_single_quoted_content() {
    let mut c = ctx();
    let r = c.html_search_meta(
        "x".to_string(),
        r#"<meta name='x' content='say "hello"'>"#.to_string(),
    );
    assert_eq!(r, Some(r#"say "hello""#.to_string()));
}

#[test]
fn og_search_property_extracts_title() {
    let mut c = ctx();
    let r = c.og_search_property(
        "title".to_string(),
        r#"<meta property="og:title" content="My Video"/>"#.to_string(),
    );
    assert_eq!(r, Some("My Video".to_string()));
}

#[test]
fn og_search_property_extracts_image() {
    let mut c = ctx();
    let r = c.og_search_property(
        "image".to_string(),
        r#"<meta property="og:image" content="https://x.com/y.jpg"/>"#.to_string(),
    );
    assert_eq!(r, Some("https://x.com/y.jpg".to_string()));
}

#[test]
fn og_search_property_returns_entities_verbatim() {
    let mut c = ctx();
    let r = c.og_search_property(
        "title".to_string(),
        r#"<meta property="og:title" content="A &amp; B"/>"#.to_string(),
    );
    // Verbatim — see `html_search_regex_returns_entities_verbatim` above.
    assert_eq!(r, Some("A &amp; B".to_string()));
}

#[test]
fn og_search_property_extracts_unquoted_value() {
    let mut c = ctx();
    let r = c.og_search_property(
        "image".to_string(),
        r#"<meta property="og:image" content=https://x.com/y.jpg />"#.to_string(),
    );
    assert_eq!(r, Some("https://x.com/y.jpg".to_string()));
}

#[test]
fn rta_search_official_meta_returns_18() {
    let mut c = ctx();
    let r =
        c.rta_search(r#"<meta name="rating" content="RTA-5042-1996-1400-1577-RTA">"#.to_string());
    assert_eq!(r, Some(18));
}

#[test]
fn rta_search_2257_marker_returns_18() {
    let mut c = ctx();
    let r = c.rta_search("footer text > 18 U.S.C. § 2257 statement".to_string());
    assert_eq!(r, Some(18));
}

#[test]
fn rta_search_no_marker_returns_none() {
    let mut c = ctx();
    let r = c.rta_search("<html><body>nothing</body></html>".to_string());
    assert_eq!(r, None);
}

#[test]
fn search_json_extracts_simple_object() {
    let mut c = ctx();
    let r = c.search_json(
        r"var data\s*=".to_string(),
        ";".to_string(),
        r#"var data = {"key": "value"};"#.to_string(),
    );
    assert_eq!(r, Some(r#"{"key": "value"}"#.to_string()));
}

#[test]
fn search_json_handles_nested() {
    let mut c = ctx();
    let r = c.search_json(
        r"var x\s*=".to_string(),
        ";".to_string(),
        r#"var x = {"a": {"b": [1,2,3]}};"#.to_string(),
    );
    assert_eq!(r, Some(r#"{"a": {"b": [1,2,3]}}"#.to_string()));
}

#[test]
fn search_json_no_match_returns_none() {
    let mut c = ctx();
    let r = c.search_json(r"NOPE".to_string(), String::new(), "irrelevant".to_string());
    assert_eq!(r, None);
}

#[tokio::test]
async fn extract_m3u8_returns_formats_via_fixture() {
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let mut c = ctx();
    let master = "\
#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n\
hi.m3u8\n";
    let fixtures = Arc::new(FetchFixtures::new().with(
        "https://x.com/master.m3u8",
        FixtureResponse::ok(master.as_bytes().to_vec()),
    ));
    c.fetch = Some(crate::host::fetch::FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(fixtures),
    });
    let opts = crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options {
        ext: None,
        protocol: None,
        m3u8_id: None,
        fatal: true,
    };
    let r = c
        .extract_m3u8(
            "https://x.com/master.m3u8".to_string(),
            "vid".to_string(),
            opts,
            empty_fetch(),
        )
        .await
        .unwrap();
    assert_eq!(r.formats.len(), 1);
    let fmt = r.formats.first().expect("len asserted above");
    assert_eq!(fmt.width, Some(1920));
}

#[test]
fn extract_json_ld_returns_typed_video() {
    let mut c = ctx();
    let html = r#"<script type="application/ld+json">
    {"@type":"VideoObject","name":"Test","description":"d","thumbnailUrl":"https://x/t.jpg","duration":"PT5M","uploadDate":"2024-01-01"}
    </script>"#;
    let r = c.extract_json_ld(html.to_string()).unwrap();
    assert_eq!(r.title, Some("Test".to_string()));
    assert_eq!(r.duration, Some(300));
}

#[tokio::test]
async fn extract_m3u8_non_fatal_swallows_fetch_failure() {
    // No FetchCtx installed → fetch returns Err("fetch capability not granted").
    // fatal=false should swallow the error and return an empty extraction.
    let mut c = ctx();
    let opts = crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options {
        ext: None,
        protocol: None,
        m3u8_id: None,
        fatal: false,
    };
    let r = c
        .extract_m3u8(
            "https://x.com/never-resolved.m3u8".to_string(),
            "vid".to_string(),
            opts,
            empty_fetch(),
        )
        .await
        .unwrap();
    assert_eq!(r.formats.len(), 0);
    assert_eq!(r.subtitles.len(), 0);
}

// ---- DASH (extract_mpd) tests ----------------------------------------

const SEGMENT_TEMPLATE_MPD: &str =
    include_str!("../../../../rdlp-downloader/tests/fixtures/dash/segment_template.mpd");
const DYNAMIC_MPD: &str =
    include_str!("../../../../rdlp-downloader/tests/fixtures/dash/dynamic.mpd");
const WITH_DRM_MPD: &str =
    include_str!("../../../../rdlp-downloader/tests/fixtures/dash/with_drm.mpd");

fn mpd_opts(fatal: bool) -> crate::bindings::rdlp::plugin::host_extract_helpers::MpdOptions {
    crate::bindings::rdlp::plugin::host_extract_helpers::MpdOptions {
        mpd_id: None,
        fatal,
    }
}

fn empty_fetch() -> crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
    crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
        headers: vec![],
        query: vec![],
        body: None,
    }
}

/// Happy-path: a valid static MPD with video and audio representations yields
/// ≥2 formats (one video-only, one audio-only), each with non-empty fragments,
/// a `fragment_base_url`, `manifest_url` matching the fetch URL, and container `"mp4_dash"`.
///
/// EXPECTED TO FAIL against the Task-4 stub (returns empty formats vec).
#[tokio::test]
async fn extract_mpd_returns_formats_via_fixture() {
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd";
    let mut c = ctx();
    c.fetch = Some(crate::host::fetch::FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(FetchFixtures::new().with(
            url,
            FixtureResponse::ok(SEGMENT_TEMPLATE_MPD.as_bytes().to_vec()),
        ))),
    });

    let r = c
        .extract_mpd(
            url.to_string(),
            "vid".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await
        .expect("extract_mpd");

    assert!(
        r.formats.len() >= 2,
        "expected ≥2 formats, got {}",
        r.formats.len()
    );
    assert!(
        r.formats
            .iter()
            .any(|f| f.vcodec.is_some() && f.acodec.is_none()),
        "expected at least one video-only format"
    );
    assert!(
        r.formats
            .iter()
            .any(|f| f.acodec.is_some() && f.vcodec.is_none()),
        "expected at least one audio-only format"
    );
    for f in &r.formats {
        assert!(
            !f.fragments.is_empty(),
            "fragments must be non-empty for {}",
            f.format_id
        );
        assert!(
            f.fragment_base_url.is_some(),
            "fragment_base_url must be set for {}",
            f.format_id
        );
        assert_eq!(
            f.manifest_url.as_deref(),
            Some(url),
            "manifest_url must match fetch URL for {}",
            f.format_id
        );
        // expand_dash_representations sets container = "{ext}_dash"
        let expected_container = format!("{}_dash", f.ext);
        assert_eq!(
            f.container.as_deref(),
            Some(expected_container.as_str()),
            "container must be {}_dash for {}",
            f.ext,
            f.format_id
        );
    }
}

/// Non-fatal path: when no `FetchCtx` is installed the fetch capability is absent.
/// With fatal=false the error must be swallowed and an empty extraction returned.
///
/// NOTE: This test MAY PASS coincidentally against the Task-4 stub, because the
/// stub returns `Ok(MpdExtraction { formats: vec![], subtitles: vec![] })` regardless
/// of input — the same value this test asserts on. The coincidental pass does NOT
/// indicate correctness; it merely means the stub and the intended behavior agree on
/// the empty-output shape for this one case. The real non-fatal path requires the
/// implementation to check for a missing `FetchCtx` and swallow the error.
#[tokio::test]
async fn extract_mpd_non_fatal_swallows_fetch_failure() {
    let mut c = ctx();
    c.fetch = None;
    let r = c
        .extract_mpd(
            "https://x/m.mpd".to_string(),
            "v".to_string(),
            mpd_opts(false),
            empty_fetch(),
        )
        .await
        .expect("non-fatal returns Ok");
    assert!(r.formats.is_empty());
    assert!(r.subtitles.is_empty());
}

/// Fatal path: when no `FetchCtx` is installed and `fatal=true` the error must propagate.
///
/// EXPECTED TO FAIL against the Task-4 stub (stub returns Ok, not Err).
#[tokio::test]
async fn extract_mpd_fatal_propagates_fetch_failure() {
    let mut c = ctx();
    c.fetch = None;
    let err = c
        .extract_mpd(
            "https://x/m.mpd".to_string(),
            "v".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await
        .expect_err("fatal must propagate");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("fetch") || msg.contains("capability"),
        "unexpected error: {msg}"
    );
}

/// A dynamic/live MPD must be refused with an error mentioning "dynamic".
///
/// EXPECTED TO FAIL against the Task-4 stub (stub returns Ok, not Err).
#[tokio::test]
async fn extract_mpd_dynamic_mpd_returns_fetch_error() {
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://x/m.mpd";
    let mut c = ctx();
    c.fetch = Some(crate::host::fetch::FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(
            FetchFixtures::new().with(url, FixtureResponse::ok(DYNAMIC_MPD.as_bytes().to_vec())),
        )),
    });
    let err = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await
        .expect_err("dynamic MPD must error");
    let msg = format!("{err:?}").to_lowercase();
    assert!(
        msg.contains("dynamic"),
        "expected 'dynamic' in error: {msg}"
    );
}

/// A DRM-protected representation must be filtered out; non-DRM sibling representations
/// must survive.
///
/// `with_drm.mpd` has DRM only on the video representation; the audio representation
/// has no `ContentProtection`. The extraction must drop the video and return only the audio.
#[tokio::test]
async fn extract_mpd_drm_protected_reps_are_dropped() {
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://x/m.mpd";
    let mut c = ctx();
    c.fetch = Some(crate::host::fetch::FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(
            FetchFixtures::new().with(url, FixtureResponse::ok(WITH_DRM_MPD.as_bytes().to_vec())),
        )),
    });
    let r = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await
        .expect("audio rep survives DRM filtering");
    assert_eq!(r.formats.len(), 1, "only the audio rep should survive");
    let f = r.formats.first().expect("len asserted above");
    assert!(f.acodec.is_some(), "expected audio codec");
    assert!(f.vcodec.is_none(), "DRM video rep must be dropped");
}

/// Non-UTF-8 bytes in the response body must cause an error (MPD is XML, must be
/// valid UTF-8 for parsing).
///
/// EXPECTED TO FAIL against the Task-4 stub (stub returns Ok, not Err).
/// Note: `FixtureResponse::ok` accepts impl `Into<Vec<u8>>`, so raw bytes work directly —
/// no `ok_bytes` variant is needed.
#[tokio::test]
async fn extract_mpd_invalid_utf8_returns_fetch_error() {
    use crate::bindings::rdlp::plugin::host_fetch::FetchError;
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://x/m.mpd";
    let mut c = ctx();
    c.fetch = Some(crate::host::fetch::FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(
            FetchFixtures::new().with(url, FixtureResponse::ok(vec![0xff_u8, 0xfe, 0xfd])),
        )),
    });
    let err = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await
        .expect_err("invalid utf-8 must error");
    assert!(
        matches!(err, FetchError::Network(_)),
        "expected FetchError::Network, got {err:?}"
    );
}

/// Well-formed XML that is not an MPD document must cause an error.
///
/// EXPECTED TO FAIL against the Task-4 stub (stub returns Ok, not Err).
#[tokio::test]
async fn extract_mpd_unparseable_xml_returns_fetch_error() {
    use crate::bindings::rdlp::plugin::host_fetch::FetchError;
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://x/m.mpd";
    let mut c = ctx();
    c.fetch = Some(crate::host::fetch::FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(
            FetchFixtures::new().with(url, FixtureResponse::ok(b"<not-mpd></not-mpd>".to_vec())),
        )),
    });
    let err = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await
        .expect_err("non-MPD XML must error");
    assert!(
        matches!(err, FetchError::Network(_)),
        "expected FetchError::Network, got {err:?}"
    );
}

// ---- fetch-options threading tests (Task 3 — expected 8 fail / 2 pass) ---

fn m3u8_opts(fatal: bool) -> crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options {
    crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options {
        ext: None,
        protocol: None,
        m3u8_id: None,
        fatal,
    }
}

fn fetch_with_headers(
    headers: Vec<(String, String)>,
) -> crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
    crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
        headers,
        query: vec![],
        body: None,
    }
}

fn fetch_with_query(
    query: Vec<(String, String)>,
) -> crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
    crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
        headers: vec![],
        query,
        body: None,
    }
}

fn fetch_with_body(
    body: Vec<u8>,
) -> crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
    crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions {
        headers: vec![],
        query: vec![],
        body: Some(body),
    }
}

#[tokio::test]
async fn extract_m3u8_threads_headers_into_request() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://example.com/playlist.m3u8";
    let mut c = ctx();
    let fixtures =
        Arc::new(FetchFixtures::new().with(url, FixtureResponse::ok(b"#EXTM3U\n".to_vec())));
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });

    let _ = c
        .extract_m3u8(
            url.to_string(),
            "v".to_string(),
            m3u8_opts(true),
            fetch_with_headers(vec![("Authorization".into(), "Bearer abc".into())]),
        )
        .await;

    let recorded = fixtures.last_request().expect("request recorded");
    assert!(
        recorded
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer abc"),
        "headers missing Authorization: {:?}",
        recorded.headers
    );
}

#[tokio::test]
async fn extract_m3u8_appends_query_params_to_url() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::FetchFixtures;
    use std::sync::Arc;

    let url = "https://example.com/playlist.m3u8";
    let mut c = ctx();
    let fixtures = Arc::new(FetchFixtures::new());
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });

    let _ = c
        .extract_m3u8(
            url.to_string(),
            "v".to_string(),
            m3u8_opts(false),
            fetch_with_query(vec![("token".into(), "abc".into())]),
        )
        .await;

    let recorded = fixtures.last_request().expect("request recorded");
    assert!(
        recorded.url.contains("?token=abc"),
        "url missing query: {}",
        recorded.url
    );
}

#[tokio::test]
async fn extract_m3u8_post_with_body() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::FetchFixtures;
    use std::sync::Arc;

    let url = "https://example.com/playlist.m3u8";
    let mut c = ctx();
    let fixtures = Arc::new(FetchFixtures::new());
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });

    let _ = c
        .extract_m3u8(
            url.to_string(),
            "v".to_string(),
            m3u8_opts(false),
            fetch_with_body(b"x".to_vec()),
        )
        .await;

    let recorded = fixtures.last_request().expect("request recorded");
    assert_eq!(
        recorded.method, "POST",
        "method should be POST when body present"
    );
}

#[tokio::test]
async fn extract_m3u8_empty_fetch_options_unchanged() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://example.com/playlist.m3u8";
    let mut c = ctx();
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(
            FetchFixtures::new().with(url, FixtureResponse::ok(b"#EXTM3U\n".to_vec())),
        )),
    });
    let _ = c
        .extract_m3u8(
            url.to_string(),
            "v".to_string(),
            m3u8_opts(false),
            empty_fetch(),
        )
        .await
        .expect("extract_m3u8 with empty fetch options");
}

#[tokio::test]
async fn extract_mpd_threads_headers_into_request() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd";
    let mut c = ctx();
    let fixtures = Arc::new(FetchFixtures::new().with(
        url,
        FixtureResponse::ok(SEGMENT_TEMPLATE_MPD.as_bytes().to_vec()),
    ));
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });
    let _ = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(true),
            fetch_with_headers(vec![("X-CDN-Token".into(), "xyz".into())]),
        )
        .await;
    let recorded = fixtures.last_request().expect("recorded");
    assert!(
        recorded
            .headers
            .iter()
            .any(|(k, v)| k == "X-CDN-Token" && v == "xyz"),
        "headers: {:?}",
        recorded.headers
    );
}

#[tokio::test]
async fn extract_mpd_appends_query_params_to_url() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::FetchFixtures;
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd";
    let mut c = ctx();
    let fixtures = Arc::new(FetchFixtures::new());
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });
    let _ = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(false),
            fetch_with_query(vec![("k".into(), "v".into())]),
        )
        .await;
    let recorded = fixtures.last_request().expect("recorded");
    assert!(recorded.url.contains("?k=v"), "url: {}", recorded.url);
}

#[tokio::test]
async fn extract_mpd_post_with_body() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::FetchFixtures;
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd";
    let mut c = ctx();
    let fixtures = Arc::new(FetchFixtures::new());
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });
    let _ = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(false),
            fetch_with_body(b"data".to_vec()),
        )
        .await;
    let recorded = fixtures.last_request().expect("recorded");
    assert_eq!(recorded.method, "POST");
}

#[tokio::test]
async fn extract_mpd_empty_fetch_options_unchanged() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd";
    let mut c = ctx();
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(FetchFixtures::new().with(
            url,
            FixtureResponse::ok(SEGMENT_TEMPLATE_MPD.as_bytes().to_vec()),
        ))),
    });
    let _ = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await;
}

#[tokio::test]
async fn query_param_with_special_chars_is_url_encoded() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::FetchFixtures;
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd";
    let mut c = ctx();
    let fixtures = Arc::new(FetchFixtures::new());
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });
    let _ = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(false),
            fetch_with_query(vec![("k".into(), "a b&c".into())]),
        )
        .await;
    let recorded = fixtures.last_request().expect("recorded");
    // Space → %20, ampersand → %26
    assert!(
        recorded.url.contains("k=a%20b%26c"),
        "url: {}",
        recorded.url
    );
}

#[tokio::test]
async fn existing_query_string_in_url_uses_ampersand_separator() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::FetchFixtures;
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd?x=1";
    let mut c = ctx();
    let fixtures = Arc::new(FetchFixtures::new());
    c.fetch = Some(FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::clone(&fixtures)),
    });
    let _ = c
        .extract_mpd(
            url.to_string(),
            "v".to_string(),
            mpd_opts(false),
            fetch_with_query(vec![("y".into(), "2".into())]),
        )
        .await;
    let recorded = fixtures.last_request().expect("recorded");
    // The new param joins with & not ?
    assert!(
        recorded.url.contains("x=1&y=2") || recorded.url.contains("y=2&x=1"),
        "url: {}",
        recorded.url
    );
}

const WITH_TEXT_TRACKS_MPD: &str =
    include_str!("../../../../rdlp-downloader/tests/fixtures/dash/with_text_tracks.mpd");

#[tokio::test]
async fn extract_mpd_returns_subtitles_via_fixture() {
    use crate::bindings::rdlp::plugin::host_extract_helpers::ExtractHelpersSubtitle;
    use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
    use std::sync::Arc;

    let url = "https://example.com/manifest.mpd";
    let mut c = ctx();
    c.fetch = Some(crate::host::fetch::FetchCtx {
        client: rdlp_http::wreq::Client::builder()
            .build()
            .expect("test client"),
        fixtures: Some(Arc::new(FetchFixtures::new().with(
            url,
            FixtureResponse::ok(WITH_TEXT_TRACKS_MPD.as_bytes().to_vec()),
        ))),
    });

    let r = c
        .extract_mpd(
            url.to_string(),
            "vid".to_string(),
            mpd_opts(true),
            empty_fetch(),
        )
        .await
        .expect("extract_mpd");

    assert_eq!(r.formats.len(), 2, "video + audio formats");
    assert_eq!(r.subtitles.len(), 3, "three subtitle reps");

    // Build a lookup by language. Lang-less sub maps to empty string at the WIT layer
    // (Python shim handles the und fallback).
    let by_lang: std::collections::HashMap<&str, &ExtractHelpersSubtitle> = r
        .subtitles
        .iter()
        .map(|s| (s.language.as_str(), s))
        .collect();
    assert!(
        by_lang.contains_key("en"),
        "en sub present: {:?}",
        by_lang.keys().collect::<Vec<_>>()
    );
    assert!(
        by_lang.contains_key("sv"),
        "sv sub present: {:?}",
        by_lang.keys().collect::<Vec<_>>()
    );
    assert!(
        by_lang.contains_key(""),
        "lang-less sub present as empty string: {:?}",
        by_lang.keys().collect::<Vec<_>>()
    );

    assert_eq!(
        by_lang.get("en").expect("en sub present").ext.as_deref(),
        Some("ttml")
    );
    assert_eq!(
        by_lang.get("sv").expect("sv sub present").ext.as_deref(),
        Some("vtt")
    );
    assert_eq!(
        by_lang
            .get("")
            .expect("lang-less sub present")
            .ext
            .as_deref(),
        Some("vtt")
    );
}

/// Regression guard for #274 (PR D-3): the `MpdFragment` WIT record must
/// carry `byte_range`, `init_url`, `init_byte_range` so plugins receive
/// DASH byte-range info (init segments + ranged media segments) from the
/// host's MPD expansion. Goes through `convert::fragment_to_wit` (the
/// crate's single `MpdFragment` construction site) rather than
/// constructing the bindgen type directly; the test still fails to compile
/// if the WIT contract drops any of the three fields, because
/// `fragment_to_wit`'s own struct literal would.
#[test]
fn mpd_fragment_carries_byte_range_fields() {
    let fr = rdlp_types::Fragment {
        url: "https://cdn.example.com/seg-0.m4s".into(),
        duration: Some(4.0),
        byte_range: Some((0u64, 1024u64)),
        init_url: Some("https://cdn.example.com/init.m4s".into()),
        init_byte_range: Some((0u64, 740u64)),
        filesize: None,
    };
    let frag = crate::convert::fragment_to_wit(fr);
    // (start, end_exclusive) convention matches rdlp_types::Fragment.
    assert_eq!(frag.byte_range, Some((0, 1024)));
    assert_eq!(
        frag.init_url.as_deref(),
        Some("https://cdn.example.com/init.m4s")
    );
    assert_eq!(frag.init_byte_range, Some((0, 740)));
}

// ---- R1: plugin-supplied patterns are bounded (rust-regex-craft) --------

/// Boundary pair, accepted side: a pattern of exactly
/// `PLUGIN_REGEX_MAX_PATTERN_LEN` bytes compiles and matches.
#[test]
fn search_regex_accepts_pattern_at_max_len() {
    let mut c = ctx();
    let pat = "a".repeat(PLUGIN_REGEX_MAX_PATTERN_LEN);
    let r = c.search_regex(pat.clone(), pat, RegexFlags::empty());
    assert_eq!(r.map(|s| s.len()), Some(PLUGIN_REGEX_MAX_PATTERN_LEN));
}

/// Boundary pair, rejected side: one byte over the cap is refused before
/// compiling, even though the pattern itself is trivially cheap.
#[test]
fn search_regex_rejects_pattern_one_over_max_len() {
    let mut c = ctx();
    let pat = "a".repeat(PLUGIN_REGEX_MAX_PATTERN_LEN + 1);
    let r = c.search_regex(pat.clone(), pat, RegexFlags::empty());
    assert_eq!(r, None);
}

/// A short pattern whose COMPILED form exceeds the cap is refused. The
/// test proves its own demonstrator: `\w{100}` builds at the crate's
/// default `size_limit` and fails at `PLUGIN_REGEX_SIZE_LIMIT`, so a
/// `None` here can only come from the bound being applied.
#[test]
fn search_regex_rejects_pattern_compiling_above_size_limit() {
    let pat = r"\w{100}";
    assert!(
        regex::RegexBuilder::new(pat).build().is_ok(),
        "demonstrator must build at the crate default"
    );
    assert!(
        regex::RegexBuilder::new(pat)
            .size_limit(PLUGIN_REGEX_SIZE_LIMIT)
            .build()
            .is_err(),
        "demonstrator must exceed PLUGIN_REGEX_SIZE_LIMIT"
    );
    let mut c = ctx();
    let r = c.search_regex(pat.to_string(), "a".repeat(100), RegexFlags::empty());
    assert_eq!(r, None);
}

/// `html_search_meta` builds its patterns from the plugin's `name`; an
/// oversized name must go through the same bound as `search_regex`.
#[test]
fn html_search_meta_rejects_oversized_name() {
    let mut c = ctx();
    let name = "a".repeat(PLUGIN_REGEX_MAX_PATTERN_LEN + 1);
    let html = format!(r#"<meta name="{name}" content="x">"#);
    assert_eq!(c.html_search_meta(name, html), None);
}

/// `og_search_property` builds its patterns from the plugin's `prop`; same
/// bound as `search_regex`.
#[test]
fn og_search_property_rejects_oversized_property() {
    let mut c = ctx();
    let prop = "a".repeat(PLUGIN_REGEX_MAX_PATTERN_LEN + 1);
    let html = format!(r#"<meta property="og:{prop}" content="x">"#);
    assert_eq!(c.og_search_property(prop, html), None);
}

/// `search_json` splices the plugin's start/end patterns in RAW, so it is
/// the helper most exposed to an expensive pattern: both bounds apply.
#[test]
fn search_json_rejects_oversized_start_pattern() {
    let mut c = ctx();
    let start = "a".repeat(PLUGIN_REGEX_MAX_PATTERN_LEN + 1);
    let hay = format!(r#"{start} {{"k":1}};"#);
    assert_eq!(c.search_json(start, ";".to_string(), hay), None);
}

#[test]
fn search_json_rejects_start_pattern_compiling_above_size_limit() {
    let mut c = ctx();
    let hay = format!(r#"{} {{"k":1}};"#, "a".repeat(100));
    assert_eq!(
        c.search_json(r"\w{100}".to_string(), ";".to_string(), hay),
        None
    );
}

// ---- D1: both manifest fetches share one fetch path ----------------------

/// The shared timeout is read at the call site for BOTH manifest helpers,
/// not merely declared: the recorded request carries it.
#[tokio::test]
async fn manifest_fetches_share_one_timeout() {
    use crate::host::fetch::FetchCtx;
    use crate::host::fetch_fixtures::FetchFixtures;
    use std::sync::Arc;

    for (is_m3u8, url) in [(true, "https://x/p.m3u8"), (false, "https://x/m.mpd")] {
        let mut c = ctx();
        let fixtures = Arc::new(FetchFixtures::new());
        c.fetch = Some(FetchCtx {
            client: rdlp_http::wreq::Client::builder()
                .build()
                .expect("test client"),
            fixtures: Some(Arc::clone(&fixtures)),
        });
        if is_m3u8 {
            let _ = c
                .extract_m3u8(
                    url.to_string(),
                    "v".to_string(),
                    m3u8_opts(false),
                    empty_fetch(),
                )
                .await;
        } else {
            let _ = c
                .extract_mpd(
                    url.to_string(),
                    "v".to_string(),
                    mpd_opts(false),
                    empty_fetch(),
                )
                .await;
        }
        let recorded = fixtures.last_request().expect("request recorded");
        assert_eq!(
            recorded.timeout_ms,
            Some(MANIFEST_FETCH_TIMEOUT_MS),
            "manifest fetch for {url} must carry the shared timeout"
        );
    }
}

// ---- refused patterns are observable ---------------------------------------

/// `(target, message)` pairs captured from the `log` facade.
type LogEntries = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// Minimal `log::Log` sink so a test can assert a refusal was reported to
/// the plugin's own log target. Mirrors the capturing-logger harness in
/// `rdlp-cookies`; `log::set_logger` accepts one logger per process, so the
/// buffer is process-global and never cleared — each assertion looks for
/// its own distinctive message instead.
struct CapturingLogger {
    entries: LogEntries,
}

impl log::Log for CapturingLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((record.target().to_string(), record.args().to_string()));
    }
    fn flush(&self) {}
}

fn captured_logs() -> LogEntries {
    static CAPTURED: std::sync::OnceLock<LogEntries> = std::sync::OnceLock::new();
    std::sync::Arc::clone(CAPTURED.get_or_init(|| {
        let entries: LogEntries = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let logger: &'static CapturingLogger = Box::leak(Box::new(CapturingLogger {
            entries: std::sync::Arc::clone(&entries),
        }));
        log::set_logger(logger).expect("no other logger in the rdlp-plugin lib test binary");
        log::set_max_level(log::LevelFilter::Warn);
        entries
    }))
}

/// First captured entry whose message contains `needle`, cloned out so the
/// lock is released before any assertion panics.
fn captured_entry_containing(logs: &LogEntries, needle: &str) -> (String, String) {
    let entries = logs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    entries
        .iter()
        .find(|(_, m)| m.contains(needle))
        .cloned()
        .unwrap_or_else(|| panic!("no entry containing {needle:?} among {entries:?}"))
}

/// A plugin author must be able to tell "pattern refused" from "no match":
/// the length-bound rejection warns on the plugin's log target with the
/// length and the limit, and never echoes the pattern.
#[test]
fn refused_over_long_pattern_warns_on_plugin_target() {
    let logs = captured_logs();
    let mut c = ctx();
    let pat = "z".repeat(PLUGIN_REGEX_MAX_PATTERN_LEN + 1);
    assert_eq!(
        c.search_regex(pat, String::new(), RegexFlags::empty()),
        None
    );
    let (target, msg) =
        captured_entry_containing(&logs, &format!("{} B", PLUGIN_REGEX_MAX_PATTERN_LEN + 1));
    assert_eq!(
        target, "plugin::test",
        "warn must go to the plugin's log target"
    );
    assert!(
        msg.contains(&PLUGIN_REGEX_MAX_PATTERN_LEN.to_string()),
        "{msg}"
    );
    assert!(
        !msg.contains("zzzz"),
        "pattern text must not be logged: {msg}"
    );
}

/// Same observability for the compiled-size bound.
#[test]
fn refused_oversize_compiled_pattern_warns_on_plugin_target() {
    let logs = captured_logs();
    let mut c = ctx();
    assert_eq!(
        c.search_regex(r"\w{100}".to_string(), String::new(), RegexFlags::empty()),
        None
    );
    let (target, msg) = captured_entry_containing(&logs, "compiled size");
    assert_eq!(target, "plugin::test");
    assert!(msg.contains(&PLUGIN_REGEX_SIZE_LIMIT.to_string()), "{msg}");
    assert!(
        !msg.contains(r"\w{100}"),
        "pattern text must not be logged: {msg}"
    );
}

// ---- expand-hls / probe-format-sizes (D2, @since 0.5.1) ----------------

mod hls_host_imports {
    use super::ctx;
    use crate::bindings::rdlp::plugin::host_extract_helpers::{FetchOptions, Host as _};
    use crate::bindings::rdlp::plugin::host_fetch::FetchError;
    use crate::host::fetch::FetchCtx;
    use crate::instance::PluginStoreData;

    /// A store with a real (non-fixture) `wreq::Client` — `expand_hls_in_place`
    /// and `detect_format_sizes_lazy_in` make genuine HTTP calls (they take an
    /// `Arc<wreq::Client>` directly, not the `host:fetch` capability), so
    /// these tests exercise them against a real mockito loopback server
    /// rather than the `FetchFixtures` canned-response map the other tests
    /// in this file use for `host:fetch` itself.
    fn store_data_with_fetch() -> PluginStoreData {
        let mut c = ctx();
        c.fetch = Some(FetchCtx {
            client: rdlp_http::wreq::Client::builder()
                .build()
                .expect("test client"),
            fixtures: None,
        });
        c
    }

    fn store_data_without_fetch() -> PluginStoreData {
        ctx()
    }

    fn wit_format(
        format_id: &str,
        url: &str,
        ext: &str,
    ) -> crate::bindings::rdlp::plugin::types::Format {
        crate::bindings::rdlp::plugin::types::Format {
            format_id: format_id.to_string(),
            url: url.to_string(),
            ext: ext.to_string(),
            protocol: ext.to_string(),
            width: None,
            height: None,
            fps: None,
            tbr: None,
            vbr: None,
            abr: None,
            vcodec: None,
            acodec: None,
            container: None,
            filesize: None,
            format_note: None,
        }
    }

    fn empty_fetch_options() -> FetchOptions {
        FetchOptions {
            headers: vec![],
            query: vec![],
            body: None,
        }
    }

    /// D2 positive: one M3u8 seed → per-variant rows carrying fragments.
    #[tokio::test]
    async fn expand_hls_returns_variant_rows_with_fragments() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::MASTER_TWO_VARIANTS)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v720.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA)
            .create_async()
            .await;
        let _v360 = server
            .mock("GET", "/v360.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA)
            .create_async()
            .await;
        let mut data = store_data_with_fetch();
        let seed = wit_format("hls", &format!("{}/master.m3u8", server.url()), "m3u8");
        let out = data
            .expand_hls(vec![seed], empty_fetch_options())
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|h| h.fragments.len() == 2));
        assert!(out.iter().any(|h| h.format.format_id == "hls-720p"));
        assert!(out.iter().all(|h| h.duration == Some(12.0)));
        // `server` must outlive every await above (its `Drop` resets the
        // mocks); this makes that live range end at its actual last point
        // of relevance rather than an implicit end-of-scope drop.
        drop(server);
    }

    /// D2 drop-not-fail: a broken variant costs only its own row.
    ///
    /// One good mockito master (2 variants) + one seed refused by the SSRF
    /// gate (`169.254.169.254` is the cloud metadata address, always
    /// rejected) + one non-HLS row. `expand_hls_in_place` drops the SSRF-
    /// refused row and passes the non-HLS row through untouched (no
    /// `fragments`); this host wrapper then filters to HLS rows only
    /// (`fragments.is_some()`), so the non-HLS row is excluded from the
    /// output too — only the good master's 2 variants remain.
    #[tokio::test]
    async fn expand_hls_drops_a_failing_seed_and_keeps_the_rest() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::MASTER_TWO_VARIANTS)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v720.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA)
            .create_async()
            .await;
        let _v360 = server
            .mock("GET", "/v360.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA)
            .create_async()
            .await;

        let mut data = store_data_with_fetch();
        let good_seed = wit_format("hls", &format!("{}/master.m3u8", server.url()), "m3u8");
        let ssrf_seed = wit_format("hls-bad", "http://169.254.169.254/x.m3u8", "m3u8");
        let mut non_hls_seed = wit_format("plain", "https://example.com/x.mp4", "mp4");
        non_hls_seed.protocol = "https".to_string();

        let out = data
            .expand_hls(
                vec![good_seed, ssrf_seed, non_hls_seed],
                empty_fetch_options(),
            )
            .await
            .unwrap();
        assert_eq!(
            out.len(),
            2,
            "only the good master's 2 variants survive: {out:?}"
        );
        drop(server);
    }

    #[tokio::test]
    async fn expand_hls_without_fetch_capability_is_fetch_error_not_trap() {
        let mut data = store_data_without_fetch();
        let err = data
            .expand_hls(
                vec![wit_format("x", "https://h/x.m3u8", "m3u8")],
                empty_fetch_options(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, FetchError::Network(ref m) if m.contains("fetch capability not granted")),
            "{err:?}"
        );
    }

    /// probe-format-sizes reuses fragments a prior expand-hls produced (no
    /// re-fetch — the master mock expects exactly 1 hit across both calls).
    #[tokio::test]
    async fn probe_format_sizes_matches_in_tree_flags_and_does_not_refetch() {
        let mut server = mockito::Server::new_async().await;
        let master = server
            .mock("GET", "/master.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::MASTER_TWO_VARIANTS)
            .expect(1)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v720.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA)
            .create_async()
            .await;
        let _v360 = server
            .mock("GET", "/v360.m3u8")
            .with_body(rdlp_extractor::hls::test_fixtures::VARIANT_MEDIA)
            .create_async()
            .await;

        let mut data = store_data_with_fetch();
        let seed = wit_format("hls", &format!("{}/master.m3u8", server.url()), "m3u8");
        let expanded = data
            .expand_hls(vec![seed], empty_fetch_options())
            .await
            .unwrap();
        let formats: Vec<_> = expanded.into_iter().map(|h| h.format).collect();
        let expected_ids: Vec<String> = formats.iter().map(|f| f.format_id.clone()).collect();

        let probe = data
            .probe_format_sizes(formats, empty_fetch_options())
            .await
            .unwrap();

        assert!(
            !probe.stream_flags.is_live,
            "{:?}",
            probe.stream_flags.is_live
        );
        assert!(
            !probe.stream_flags.has_any_drm,
            "{:?}",
            probe.stream_flags.has_any_drm
        );
        assert_eq!(
            probe
                .formats
                .iter()
                .map(|f| f.format_id.clone())
                .collect::<Vec<_>>(),
            expected_ids
        );
        master.assert_async().await;
        drop(server);
    }

    #[tokio::test]
    async fn probe_format_sizes_reports_live_and_drm_flags() {
        let mut server = mockito::Server::new_async().await;
        let _live = server
            .mock("GET", "/live.m3u8")
            .with_body(
                "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXTINF:6.0,\n\
seg-1.ts\n",
            )
            .create_async()
            .await;
        let _drm = server
            .mock("GET", "/drm.m3u8")
            .with_body(
                "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-KEY:METHOD=AES-128,URI=\"https://h.com/key\"\n\
#EXTINF:6.0,\n\
seg-1.ts\n\
#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let mut data = store_data_with_fetch();
        let seeds = vec![
            wit_format("live", &format!("{}/live.m3u8", server.url()), "m3u8"),
            wit_format("drm", &format!("{}/drm.m3u8", server.url()), "m3u8"),
        ];
        let probe = data
            .probe_format_sizes(seeds, empty_fetch_options())
            .await
            .unwrap();
        assert!(probe.stream_flags.is_live, "{:?}", probe.stream_flags);
        assert!(probe.stream_flags.has_any_drm, "{:?}", probe.stream_flags);
        drop(server);
    }
}
