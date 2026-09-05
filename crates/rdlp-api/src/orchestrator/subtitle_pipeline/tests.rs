//! Tests for subtitle pipeline: validation, normalization, and policy
#![allow(clippy::indexing_slicing)]

use super::*;
use rdlp_types::SubtitleKind;

fn track(lang: &str, ext: &str, is_auto: bool) -> SubtitleTrack {
    SubtitleTrack {
        language: lang.to_string(),
        url: format!("https://example.com/{lang}.{ext}"),
        ext: ext.to_string(),
        is_auto,
        kind: SubtitleKind::Normal,
        name: None,
    }
}

fn result_with(tracks: Vec<SubtitleTrack>) -> SubtitleResult {
    SubtitleResult::available(tracks)
}

// -- Policy tests --

#[test]
fn test_no_requested_langs() {
    let result = result_with(vec![track("en", "srt", false)]);
    let outcome = apply_subtitle_policy(&result, &[], None, false, false);
    assert!(outcome.selected.is_empty());
    assert!(outcome.warnings.is_empty());
    assert!(!outcome.should_fail);
}

#[test]
fn test_requested_lang_found_manual() {
    let result = result_with(vec![track("en", "srt", false)]);
    let langs = vec!["en".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, false, false);
    assert_eq!(outcome.selected.len(), 1);
    assert_eq!(outcome.selected[0].language, "en");
    assert!(outcome.warnings.is_empty());
}

#[test]
fn test_requested_lang_auto_only_include_auto() {
    let result = result_with(vec![track("en", "vtt", true)]);
    let langs = vec!["en".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, true, false);
    assert_eq!(outcome.selected.len(), 1);
    assert!(outcome.selected[0].is_auto);
}

#[test]
fn test_requested_lang_auto_only_exclude_auto() {
    let result = result_with(vec![track("en", "vtt", true)]);
    let langs = vec!["en".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, false, false);
    assert!(outcome.selected.is_empty());
    assert_eq!(outcome.warnings.len(), 1);
    assert!(!outcome.should_fail);
}

#[test]
fn test_requested_lang_missing_lenient() {
    let result = result_with(vec![track("ja", "srt", false)]);
    let langs = vec!["en".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, false, false);
    assert!(outcome.selected.is_empty());
    assert_eq!(outcome.warnings.len(), 1);
    assert!(!outcome.should_fail);
}

#[test]
fn test_requested_lang_missing_strict() {
    let result = result_with(vec![track("ja", "srt", false)]);
    let langs = vec!["en".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, false, true);
    assert!(outcome.selected.is_empty());
    assert!(outcome.should_fail);
    assert!(outcome.error_message.is_some());
}

#[test]
fn test_all_keyword() {
    let result = result_with(vec![
        track("en", "srt", false),
        track("ja", "vtt", false),
        track("es", "srt", true),
    ]);
    let langs = vec!["all".to_string()];
    // include_auto = true -> all 3 tracks
    let outcome = apply_subtitle_policy(&result, &langs, None, true, false);
    assert_eq!(outcome.selected.len(), 3);
}

#[test]
fn test_preferred_format_respected() {
    let result = result_with(vec![track("en", "vtt", false), track("en", "srt", false)]);
    let langs = vec!["en".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, Some(SubtitleFormat::Srt), false, false);
    assert_eq!(outcome.selected.len(), 1);
    assert_eq!(outcome.selected[0].ext, "srt");
}

#[test]
fn test_format_fallback_srt_over_vtt() {
    let result = result_with(vec![track("en", "vtt", false), track("en", "srt", false)]);
    let langs = vec!["en".to_string()];
    // No preferred format -> fallback order prefers srt
    let outcome = apply_subtitle_policy(&result, &langs, None, false, false);
    assert_eq!(outcome.selected.len(), 1);
    assert_eq!(outcome.selected[0].ext, "srt");
}

// -- Language matching tests --

#[test]
fn test_track_matches_lang_exact() {
    let t = track("en", "srt", false);
    assert!(track_matches_lang(&t, "en"));
    assert!(track_matches_lang(&t, "EN"));
}

#[test]
fn test_track_matches_lang_prefix() {
    let t = track("English", "vtt", false);
    assert!(track_matches_lang(&t, "en"));
    assert!(track_matches_lang(&t, "En"));
    assert!(!track_matches_lang(&t, "es"));
}

#[test]
fn test_track_matches_lang_name_field() {
    let mut t = track("en", "srt", false);
    t.name = Some("English".to_string());
    assert!(track_matches_lang(&t, "English"));
    assert!(track_matches_lang(&t, "en"));
}

#[test]
fn test_track_matches_lang_no_false_positive() {
    let t = track("Japanese", "vtt", false);
    assert!(!track_matches_lang(&t, "en"));
    // "ja" should match "Japanese"
    assert!(track_matches_lang(&t, "ja"));
}

#[test]
fn test_policy_iso_code_matches_label() {
    // Simulates 9anime: tracks keyed by "English" label,
    // user passes --sub-langs en
    let result = result_with(vec![track("English", "vtt", false)]);
    let langs = vec!["en".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, false, false);
    assert_eq!(outcome.selected.len(), 1);
    assert_eq!(outcome.selected[0].language, "English");
    assert!(outcome.warnings.is_empty());
}

#[test]
fn test_strict_subs_error_message_content() {
    let result = result_with(vec![track("ja", "srt", false)]);
    let langs = vec!["en".to_string(), "de".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, false, true);
    assert!(outcome.should_fail);
    let msg = outcome.error_message.unwrap();
    assert!(msg.contains("2 language(s) unavailable"));
}

#[test]
fn test_strict_subs_passes_when_all_found() {
    let result = result_with(vec![track("en", "srt", false), track("ja", "vtt", false)]);
    let langs = vec!["en".to_string(), "ja".to_string()];
    let outcome = apply_subtitle_policy(&result, &langs, None, false, true);
    assert!(!outcome.should_fail);
    assert_eq!(outcome.selected.len(), 2);
    assert!(outcome.warnings.is_empty());
}

// -- URL validation tests --

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_validate_urls_keeps_reachable() {
    let opts = mockito::ServerOpts {
        host: "127.0.0.1",
        ..Default::default()
    };
    let mut server = mockito::Server::new_with_opts_async(opts).await;
    let base_url = server.url();
    let _mock = server
        .mock("HEAD", "/en.vtt")
        .with_status(200)
        .expect(1)
        .create_async()
        .await;

    let mut t = track("en", "vtt", false);
    t.url = base_url + "/en.vtt";
    let result = result_with(vec![t]);

    let client = wreq::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let validated = validate_subtitle_urls(result, &client).await;

    assert_eq!(validated.tracks.len(), 1);
    assert_eq!(validated.status, SubtitleStatus::Available);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_validate_urls_filters_unreachable() {
    let opts = mockito::ServerOpts {
        host: "127.0.0.1",
        ..Default::default()
    };
    let mut server = mockito::Server::new_with_opts_async(opts).await;
    let base_url = server.url();
    let _mock = server
        .mock("HEAD", "/en.vtt")
        .with_status(404)
        .expect(1)
        .create_async()
        .await;

    let mut t = track("en", "vtt", false);
    t.url = base_url + "/en.vtt";
    let result = result_with(vec![t]);

    let client = wreq::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let validated = validate_subtitle_urls(result, &client).await;

    assert!(validated.tracks.is_empty());
    assert_eq!(validated.status, SubtitleStatus::ErrorSoft);
    assert!(
        validated
            .reasons
            .contains(&SubtitleReason::UrlNotReachable(404))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_validate_urls_auth_reason_on_403() {
    let opts = mockito::ServerOpts {
        host: "127.0.0.1",
        ..Default::default()
    };
    let mut server = mockito::Server::new_with_opts_async(opts).await;
    let base_url = server.url();
    let _mock = server
        .mock("HEAD", "/en.vtt")
        .with_status(403)
        .expect(1)
        .create_async()
        .await;

    let mut t = track("en", "vtt", false);
    t.url = base_url + "/en.vtt";
    let result = result_with(vec![t]);

    let client = wreq::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let validated = validate_subtitle_urls(result, &client).await;

    assert!(validated.tracks.is_empty());
    assert!(validated.reasons.contains(&SubtitleReason::RequiresAuth));
}

/// The vector this branch exists to close, at the one place reachable without
/// a live server.
///
/// Measured against wreq 6.0.0-rc.28: its `Error` Display renders
/// `error sending request for uri (<uri>)`, and for a request built through
/// `RequestBuilder` it STRIPS userinfo from that uri — but keeps the query
/// string intact. So `admin:hunter2@` is not the leak on this path; `?token=…`
/// is, and that is how signed CDN URLs actually carry credentials. A first
/// version of this test used userinfo and passed with the redaction removed,
/// proving nothing.
///
/// The strip is a property of the builder path, not of wreq errors generally:
/// an error raised while following a redirect carries the target taken from
/// the `Location` header, which never passes through that stripping. So
/// userinfo CAN appear there — `USERINFO_PATTERN` covers it, and the
/// redaction here is what makes that moot.
///
/// Port 1 refuses immediately, so the error is real with no mock and no
/// network. `validate_single_url` is a plain private async fn taking a
/// `&wreq::Client` — the seam I had wrongly reported as absent.
#[tokio::test(flavor = "current_thread")]
async fn a_network_error_carries_no_credentials_from_the_url() {
    let client = wreq::Client::builder().build().expect("client builds");
    let outcome = super::validate_single_url(
        "http://127.0.0.1:1/subs.vtt?token=hunter2&expires=1",
        &client,
    )
    .await;

    let super::UrlValidation::NetworkError(message) = outcome else {
        panic!("a refused connection must produce NetworkError");
    };
    assert!(
        !message.contains("hunter2"),
        "signed-URL token survived into the diagnostic/log string: {message}"
    );
    // The host and path must survive, or the diagnostic stops saying where.
    assert!(message.contains("127.0.0.1"), "over-redacted: {message}");
}
