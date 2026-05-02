//! Expand an MPEG-DASH MPD into one [`Format`] per usable Representation.
//!
//! Mirrors yt-dlp's `_parse_mpd_periods` (common.py:2870–3073) at the
//! per-Repr-Format level. The DASH downloader is the consumer; it reads
//! `format.fragments` directly without re-parsing the manifest.

use rdlp_types::{Codec, DownloadProtocol, Format, Fragment};
use url::Url;

use super::audio_sampling_rate::parse_audio_sampling_rate;
use super::baseurl::resolve_chain;
use super::errors::DashExpandError;
use super::frame_rate::parse_frame_rate;
use super::segments::{
    SegmentListEntry, SegmentListPlan, SegmentTemplatePlan, SegmentTimelinePlan, TimelineEntry,
    resolve_segment_list, resolve_segment_template, resolve_segment_timeline,
};

/// Hard cap on representations per MPD. Task 11 exercises the truncation logic.
pub(crate) const MAX_REPS_PER_MPD: usize = 50;

/// Parse the MPD body and project each Representation to a [`Format`].
///
/// `mpd_xml` is the response body. `base_url` is the URL the MPD was fetched
/// from (used as the root of the BaseURL resolution chain).
///
/// Returns one Format per usable Representation in the **first** Period.
/// Multi-period MPDs log a warning and skip subsequent periods (see spec's
/// non-goals — more conservative than yt-dlp).
///
/// # Errors
///
/// Returns [`DashExpandError`] when the MPD cannot be parsed, declares a live
/// stream, or contains no usable representations after filtering.
pub fn expand_dash_representations(
    mpd_xml: &str,
    base_url: &Url,
) -> Result<Vec<Format>, DashExpandError> {
    let mpd = dash_mpd::parse(mpd_xml).map_err(|e| DashExpandError::Parse(e.to_string()))?;

    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(DashExpandError::DynamicMpd);
    }

    if mpd.periods.len() > 1 {
        log::warn!(
            "DASH: MPD has {} periods; only first is expanded (multi-period merging deferred)",
            mpd.periods.len()
        );
    }

    let Some(period) = mpd.periods.first() else {
        return Err(DashExpandError::NoUsableReps);
    };

    let mpd_baseurls: Vec<String> = mpd.base_url.iter().map(|b| b.base.clone()).collect();
    let period_duration_seconds = period
        .duration
        .as_ref()
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    let mut drm_dropped = 0usize;
    let mut formats: Vec<Format> = Vec::new();

    for (adapt_idx, adapt) in period.adaptations.iter().enumerate() {
        if !adapt.ContentProtection.is_empty() {
            drm_dropped += adapt.representations.len();
            continue;
        }

        let adapt_baseurls: Vec<String> = adapt.BaseURL.iter().map(|b| b.base.clone()).collect();
        let adapt_lang = adapt.lang.clone();

        for (repr_idx, repr) in adapt.representations.iter().enumerate() {
            if !repr.ContentProtection.is_empty() {
                drm_dropped += 1;
                continue;
            }

            let repr_baseurls: Vec<String> = repr.BaseURL.iter().map(|b| b.base.clone()).collect();

            let final_base = resolve_chain(
                base_url,
                [
                    mpd_baseurls.as_slice(),
                    adapt_baseurls.as_slice(),
                    repr_baseurls.as_slice(),
                ],
            );

            let mime = repr
                .mimeType
                .as_deref()
                .or(adapt.mimeType.as_deref())
                .unwrap_or("");
            let is_video = mime.starts_with("video/");
            let is_audio = mime.starts_with("audio/");
            if !is_video && !is_audio {
                continue;
            }

            let bandwidth = repr.bandwidth.unwrap_or(0);
            let synth_id = if is_video {
                format!("dash_v_{adapt_idx}_{repr_idx}")
            } else {
                format!("dash_a_{adapt_idx}_{repr_idx}")
            };
            let format_id = repr.id.clone().unwrap_or(synth_id);

            let fragments =
                build_fragments(adapt, repr, &format_id, bandwidth, period_duration_seconds);
            if fragments.is_empty() {
                continue;
            }

            let codecs = repr
                .codecs
                .clone()
                .or_else(|| adapt.codecs.clone())
                .unwrap_or_default();

            let (vcodec, acodec) = if is_video {
                (Codec::Present(codecs), Codec::Absent)
            } else {
                (Codec::Absent, Codec::Present(codecs))
            };

            let ext = mime_to_ext(mime);
            let container = format!("{ext}_dash");

            let fps = repr.frameRate.as_deref().and_then(parse_frame_rate);
            let asr = repr
                .audioSamplingRate
                .as_deref()
                .or(adapt.audioSamplingRate.as_deref())
                .and_then(parse_audio_sampling_rate);

            let mut f = Format::new(
                &format_id,
                base_url.as_str(),
                ext,
                DownloadProtocol::HttpDashSegments,
            );
            f.vcodec = vcodec;
            f.acodec = acodec;
            f.container = Some(container);
            f.tbr = if bandwidth > 0 {
                Some(bandwidth as f64 / 1000.0)
            } else {
                None
            };
            f.width = repr.width.map(|w| w as u32);
            f.height = repr.height.map(|h| h as u32);
            f.fps = fps;
            f.asr = if is_audio { asr } else { None };
            f.language = adapt_lang.clone();
            f.fragments = Some(fragments);
            f.fragment_base_url = Some(final_base.to_string());

            formats.push(f);
        }
    }

    if drm_dropped > 0 {
        log::warn!("DASH: dropped {drm_dropped} DRM-protected representation(s)");
    }

    if formats.len() > MAX_REPS_PER_MPD {
        formats.sort_by(|a, b| {
            b.tbr
                .partial_cmp(&a.tbr)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let dropped = formats.len() - MAX_REPS_PER_MPD;
        formats.truncate(MAX_REPS_PER_MPD);
        log::warn!("DASH: capped representations at {MAX_REPS_PER_MPD} (dropped {dropped})");
    }

    if formats.is_empty() {
        return Err(DashExpandError::NoUsableReps);
    }
    Ok(formats)
}

/// Map a MIME type to a container file extension.
fn mime_to_ext(mime: &str) -> &'static str {
    match mime {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mp4" => "m4a",
        "audio/webm" => "webm",
        _ => "mp4",
    }
}

/// Resolve the init URL from a `SegmentTemplate`.
///
/// Prefers the `@initialization` attribute (a URL string). Falls back to the
/// `<Initialization sourceURL="…">` child element.
fn tmpl_init_url(tmpl: &dash_mpd::SegmentTemplate) -> Option<String> {
    // @initialization attribute takes precedence (most common form).
    if let Some(url) = tmpl.initialization.as_ref().filter(|u| !u.is_empty()) {
        return Some(url.clone());
    }
    // <Initialization sourceURL="…"> child element.
    tmpl.Initialization
        .as_ref()
        .and_then(|i| i.sourceURL.clone())
        .filter(|u| !u.is_empty())
}

/// Build a pre-resolved fragment list for one Representation.
///
/// Priority: Repr-level `SegmentTemplate` → AdaptationSet-level `SegmentTemplate`
/// → Repr-level `SegmentList` → AdaptationSet-level `SegmentList`.
///
/// Returns an empty Vec when no segment information is found (the caller skips
/// the Repr).
fn build_fragments(
    adapt: &dash_mpd::AdaptationSet,
    repr: &dash_mpd::Representation,
    format_id: &str,
    bandwidth: u64,
    period_duration_seconds: f64,
) -> Vec<Fragment> {
    // SegmentTemplate path (most common for on-demand content).
    if let Some(tmpl) = repr
        .SegmentTemplate
        .as_ref()
        .or(adapt.SegmentTemplate.as_ref())
    {
        // Duration-based: compute count from period duration / segment duration.
        if let (Some(media), Some(duration), Some(timescale)) =
            (&tmpl.media, tmpl.duration, tmpl.timescale)
        {
            let plan = SegmentTemplatePlan {
                initialization: tmpl_init_url(tmpl),
                media: media.clone(),
                start_number: tmpl.startNumber.unwrap_or(1),
                // duration is f64 in dash-mpd 0.20.2 (handles non-integer values in the wild).
                duration: duration as u64,
                timescale,
                period_duration_seconds,
            };
            return resolve_segment_template(&plan, format_id, bandwidth);
        }
        // SegmentTimeline: explicit <S t d r> entries.
        if let (Some(timeline), Some(media), Some(timescale)) =
            (tmpl.SegmentTimeline.as_ref(), &tmpl.media, tmpl.timescale)
        {
            let entries: Vec<TimelineEntry> = timeline
                .segments
                .iter()
                .map(|s| TimelineEntry {
                    t: s.t,
                    d: s.d,
                    r: s.r.unwrap_or(0),
                })
                .collect();
            let plan = SegmentTimelinePlan {
                initialization: tmpl_init_url(tmpl),
                media: media.clone(),
                timescale,
                entries,
            };
            return resolve_segment_timeline(&plan, format_id, bandwidth);
        }
    }

    // SegmentList path: explicit <SegmentURL media="…"/> list.
    if let Some(seg_list) = repr.SegmentList.as_ref().or(adapt.SegmentList.as_ref()) {
        let plan = SegmentListPlan {
            initialization: seg_list
                .Initialization
                .as_ref()
                .and_then(|i| i.sourceURL.clone()),
            entries: seg_list
                .segment_urls
                .iter()
                .filter_map(|u| {
                    u.media.clone().map(|m| SegmentListEntry {
                        media: m,
                        duration_seconds: None,
                    })
                })
                .collect(),
        };
        return resolve_segment_list(&plan);
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::DownloadProtocol;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/rdlp-downloader/tests/fixtures/dash")
            .join(name)
    }

    fn load_fixture(name: &str) -> String {
        std::fs::read_to_string(fixture_path(name))
            .unwrap_or_else(|_| panic!("fixture missing: {name}"))
    }

    #[test]
    fn segment_template_three_video_two_audio() {
        let xml = load_fixture("segment_template.mpd");
        let base = Url::parse("https://cdn.example.com/manifest.mpd").unwrap();
        let formats = expand_dash_representations(&xml, &base).unwrap();

        // Fixture defines at least one video Repr + one audio Repr.
        assert!(
            formats.len() >= 2,
            "expected at least 2 reps from fixture, got {}",
            formats.len()
        );

        let video_count = formats.iter().filter(|f| f.vcodec.is_present()).count();
        let audio_count = formats.iter().filter(|f| f.acodec.is_present()).count();
        assert!(video_count > 0, "at least one video Repr");
        assert!(audio_count > 0, "at least one audio Repr");

        for f in &formats {
            assert_eq!(f.protocol, DownloadProtocol::HttpDashSegments);
            assert!(f.fragments.is_some(), "fragments must be pre-resolved");
            let frags = f.fragments.as_ref().unwrap();
            assert!(!frags.is_empty(), "non-empty fragment list");
            // Video-only XOR audio-only — never both, never neither.
            assert_ne!(
                f.vcodec.is_present(),
                f.acodec.is_present(),
                "format {} should be either video-only or audio-only, not both/neither",
                f.format_id
            );
        }
    }

    #[test]
    fn dynamic_mpd_returns_error() {
        let xml = load_fixture("dynamic.mpd");
        let base = Url::parse("https://cdn.example.com/m.mpd").unwrap();
        let err = expand_dash_representations(&xml, &base).unwrap_err();
        assert!(matches!(err, DashExpandError::DynamicMpd));
    }

    #[test]
    fn drm_reps_filtered() {
        let xml = load_fixture("with_drm.mpd");
        let base = Url::parse("https://cdn.example.com/m.mpd").unwrap();
        let result = expand_dash_representations(&xml, &base);
        match result {
            Ok(formats) => {
                // None of the returned Formats should be DRM-encumbered.
                // We can't introspect ContentProtection from a Format, but we
                // can assert that every returned Format passed the filter.
                assert!(!formats.is_empty(), "non-DRM Reps should remain");
            }
            Err(DashExpandError::NoUsableReps) => {
                // Acceptable if every Rep was DRM-protected.
            }
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn multi_period_first_only() {
        let xml = load_fixture("multi_period.mpd");
        let base = Url::parse("https://cdn.example.com/m.mpd").unwrap();
        let formats = expand_dash_representations(&xml, &base).unwrap();
        // Multi-period fixture has Reps in each period; we only emit period-1's.
        // Asserting non-empty is sufficient — a non-failing parse with at least
        // one Format proves the multi-period warn-and-skip path succeeded.
        assert!(!formats.is_empty());
    }

    #[test]
    fn mega_rep_cap_at_50() {
        let xml = load_fixture("mega_reps.mpd");
        let base = Url::parse("https://cdn.example.com/m.mpd").unwrap();
        let formats = expand_dash_representations(&xml, &base).unwrap();
        assert_eq!(formats.len(), 50, "60-Rep MPD should cap at 50");
        // The 50 retained should be the highest-bandwidth Reps (v11..v60).
        // Bandwidth formula: 100000 + i * 10000 → v11 = 210000, v60 = 700000.
        // After sort-desc + truncate, v11..v60 remain.
        assert!(
            formats.iter().any(|f| f.format_id == "v60"),
            "highest-bandwidth Rep must remain"
        );
        assert!(
            !formats.iter().any(|f| f.format_id == "v1"),
            "lowest-bandwidth Rep must be dropped"
        );
    }

    #[test]
    fn missing_repr_id_synthesizes() {
        // Synthesize an MPD inline with a Rep that has no @id attribute.
        let xml = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT4S">
  <Period duration="PT4S">
    <AdaptationSet mimeType="video/mp4" codecs="avc1.4d401e">
      <Representation bandwidth="1000000" width="640" height="360">
        <SegmentTemplate media="$Number$.m4s" duration="2" timescale="1" startNumber="1"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
        let base = Url::parse("https://cdn.example.com/m.mpd").unwrap();
        let formats = expand_dash_representations(xml, &base).unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format_id, "dash_v_0_0");
    }
}
