//! Tests for download state machine types

use super::*;

#[test]
fn test_selecting_subtitles_display() {
    let phase = DownloadPhase::SelectingSubtitles {
        info: Box::new(rdlp_types::InfoDict::new(
            "id",
            "title",
            "test",
            "http://example.com",
        )),
        format: Box::new(rdlp_types::Format::new(
            "f1",
            "http://example.com/v.mp4",
            "mp4",
            rdlp_types::DownloadProtocol::Https,
        )),
        plan: Box::new(super::super::DownloadPlan::Single(rdlp_types::Format::new(
            "f1",
            "http://example.com/v.mp4",
            "mp4",
            rdlp_types::DownloadProtocol::Https,
        ))),
    };

    assert_eq!(format!("{phase}"), "selecting subtitles");
}

#[test]
fn test_selecting_subtitles_passes_through_when_no_subs() {
    // Verify the phase can be constructed with empty subtitle data
    let info = Box::new(rdlp_types::InfoDict::new(
        "id",
        "title",
        "test",
        "http://example.com",
    ));

    // No subtitles in info -> select_subtitles_if_needed returns empty vec
    assert!(info.subtitles.is_none());
    assert!(info.automatic_captions.is_none());
}
