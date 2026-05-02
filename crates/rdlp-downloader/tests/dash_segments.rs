//! Unit tests for `rdlp_downloader::dash::segments::SegmentPlan`.

#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    clippy::expect_used
)]

use rdlp_downloader::dash::segments::{
    SegmentListPlan, SegmentPlan, SegmentTemplatePlan, SegmentTimelinePlan, TimelineEntry,
};
use url::Url;

fn base() -> Vec<Url> {
    vec![Url::parse("https://example.test/dash/").unwrap()]
}

#[test]
fn template_substitutes_number_and_bandwidth() {
    let plan = SegmentPlan::Template(SegmentTemplatePlan {
        init: Some("video/init.mp4".into()),
        media: "video/$RepresentationID$/$Number$-$Bandwidth$.m4s".into(),
        start_number: 1,
        timescale: 1000,
        segment_duration_ts: 6000,
        total_segments: 3,
        representation_id: "v1".into(),
        bandwidth: 1_000_000,
    });
    let urls = plan.segment_urls(&base(), 18_000);
    assert_eq!(urls.len(), 3);
    assert_eq!(
        urls[0].as_str(),
        "https://example.test/dash/video/v1/1-1000000.m4s"
    );
    assert_eq!(
        urls[2].as_str(),
        "https://example.test/dash/video/v1/3-1000000.m4s"
    );
}

#[test]
fn timeline_expands_positive_repeats() {
    let plan = SegmentPlan::Timeline(SegmentTimelinePlan {
        init: None,
        media: "seg-$Number$.m4s".into(),
        start_number: 1,
        timescale: 1000,
        entries: vec![TimelineEntry {
            t: Some(0),
            d: 6000,
            r: Some(4),
        }], // 5 segments
        representation_id: "v1".into(),
        bandwidth: 1_000_000,
    });
    let urls = plan.segment_urls(&base(), 30_000);
    assert_eq!(urls.len(), 5);
    assert_eq!(urls[0].as_str(), "https://example.test/dash/seg-1.m4s");
    assert_eq!(urls[4].as_str(), "https://example.test/dash/seg-5.m4s");
}

#[test]
fn timeline_expands_negative_r_to_period_end() {
    // Period = 30s @ 1000 timescale = 30_000 ts. Each segment 6000 ts → 5 segments.
    let plan = SegmentPlan::Timeline(SegmentTimelinePlan {
        init: None,
        media: "seg-$Number$.m4s".into(),
        start_number: 1,
        timescale: 1000,
        entries: vec![TimelineEntry {
            t: Some(0),
            d: 6000,
            r: Some(-1),
        }],
        representation_id: "v1".into(),
        bandwidth: 1,
    });
    let urls = plan.segment_urls(&base(), 30_000);
    assert_eq!(urls.len(), 5);
}

#[test]
fn timeline_no_repeat_emits_single_segment() {
    let plan = SegmentPlan::Timeline(SegmentTimelinePlan {
        init: None,
        media: "seg-$Number$.m4s".into(),
        start_number: 1,
        timescale: 1000,
        entries: vec![TimelineEntry {
            t: Some(0),
            d: 6000,
            r: None,
        }],
        representation_id: "v1".into(),
        bandwidth: 1,
    });
    assert_eq!(plan.segment_urls(&base(), 30_000).len(), 1);
}

#[test]
fn list_resolves_relative_urls() {
    let plan = SegmentPlan::List(SegmentListPlan {
        init: Some("init.mp4".into()),
        urls: vec!["s1.m4s".into(), "s2.m4s".into()],
    });
    let urls = plan.segment_urls(&base(), 0);
    assert_eq!(urls.len(), 2);
    assert_eq!(urls[0].as_str(), "https://example.test/dash/s1.m4s");
}

#[test]
fn init_url_resolves() {
    let plan = SegmentPlan::Template(SegmentTemplatePlan {
        init: Some("init.mp4".into()),
        media: "seg-$Number$.m4s".into(),
        start_number: 1,
        timescale: 1000,
        segment_duration_ts: 6000,
        total_segments: 1,
        representation_id: "v1".into(),
        bandwidth: 1,
    });
    assert_eq!(
        plan.init_url(&base()).unwrap().as_str(),
        "https://example.test/dash/init.mp4"
    );
}
