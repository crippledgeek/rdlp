//! Integration tests for `rdlp_downloader::dash::manifest::parse_mpd`.

#![allow(clippy::disallowed_methods, clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use url::Url;

fn fixture(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/dash");
    p.push(name);
    std::fs::read_to_string(&p).expect("fixture exists")
}

fn base() -> Url {
    Url::parse("https://example.test/dash/manifest.mpd").unwrap()
}

#[test]
fn segment_template_picks_highest_bandwidth_video_and_audio() {
    let body = fixture("segment_template.mpd");
    let m = rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap();
    assert_eq!(m.video.id, "v1");
    assert_eq!(m.video.bandwidth, 1_000_000);
    assert_eq!(m.audio.as_ref().unwrap().id, "a1");
    assert_eq!(m.video.width, Some(1920));
    assert_eq!(m.video.height, Some(1080));
}

#[test]
fn segment_timeline_parses() {
    let body = fixture("segment_timeline.mpd");
    rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap();
}

#[test]
fn segment_list_parses() {
    let body = fixture("segment_list.mpd");
    rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap();
}

#[test]
fn dynamic_mpd_refused() {
    let body = fixture("dynamic.mpd");
    let err = rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap_err();
    assert!(matches!(err, rdlp_downloader::dash::DashError::DynamicMpd));
}

#[test]
fn drm_mpd_refused() {
    let body = fixture("with_drm.mpd");
    let err = rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap_err();
    assert!(matches!(
        err,
        rdlp_downloader::dash::DashError::DrmProtected
    ));
}

#[test]
fn multi_period_warns_and_returns_first_period() {
    let body = fixture("multi_period.mpd");
    let m = rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap();
    assert!(m.period_count >= 2);
}

#[test]
fn segment_template_fixture_produces_template_plan() {
    let body = fixture("segment_template.mpd");
    let m = rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap();
    match &m.video.plan {
        rdlp_downloader::dash::segments::SegmentPlan::Template(t) => {
            assert_eq!(t.start_number, 1);
            assert_eq!(t.timescale, 1000);
            // 30s @ 6000-tick segments / 1000 timescale → 5 segments.
            assert_eq!(t.total_segments, 5);
            assert_eq!(t.representation_id, "v1");
        }
        other => panic!("expected Template plan, got {other:?}"),
    }
}

#[test]
fn segment_timeline_fixture_produces_timeline_plan() {
    let body = fixture("segment_timeline.mpd");
    let m = rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap();
    assert!(matches!(
        m.video.plan,
        rdlp_downloader::dash::segments::SegmentPlan::Timeline(_)
    ));
}

#[test]
fn segment_list_fixture_produces_list_plan() {
    let body = fixture("segment_list.mpd");
    let m = rdlp_downloader::dash::manifest::parse_mpd(&body, &base()).unwrap();
    assert!(matches!(
        m.video.plan,
        rdlp_downloader::dash::segments::SegmentPlan::List(_)
    ));
}
