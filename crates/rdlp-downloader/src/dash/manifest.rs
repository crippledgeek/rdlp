//! MPD parser. Wraps `dash_mpd::parse` and projects the relevant subset
//! into rdlp-internal types.
//!
//! Refuses live (`@type="dynamic"`) and DRM-protected MPDs. Multi-period
//! MPDs parse with a warning; only the first period is consumed (Task 3+
//! may revisit). The actual `SegmentPlan` is attached in Task 3 — this
//! module only resolves representation selection and BaseURL chains.

// DASH proper nouns (MPD, AdaptationSet, Representation, BaseURL, Period)
// appear extensively in doc-comments. The pedantic doc_markdown lint flags
// them as bare CamelCase identifiers, which is noise for a module whose
// vocabulary is fixed by an external spec.
#![allow(clippy::doc_markdown)]

use std::time::Duration;

use rdlp_types::Rfc6381Codec;
use url::Url;

use crate::dash::errors::DashError;
use crate::dash::segments::{
    SegmentListPlan, SegmentPlan, SegmentTemplatePlan, SegmentTimelinePlan, TimelineEntry,
};

/// Result of parsing an MPD manifest. Owns enough state to drive segment
/// downloads in a later task — the actual `SegmentPlan` is attached in Task 3.
#[derive(Debug)]
pub struct ParsedManifest {
    /// Effective duration of the (first) period. Falls back to
    /// `mpd@mediaPresentationDuration`, then to `Duration::ZERO`.
    pub period_duration: Duration,
    /// Selected video representation (highest bandwidth in the first period).
    pub video: RepresentationInfo,
    /// Optional audio representation (None if no audio AdaptationSet exists).
    pub audio: Option<RepresentationInfo>,
    /// MPD-level resolved BaseURL chain. Always non-empty: at minimum it
    /// contains the URL the MPD was fetched from.
    pub mpd_base_urls: Vec<Url>,
    /// Total number of `<Period>` elements in the source MPD (>=1).
    pub period_count: usize,
}

/// Resolved metadata for a single Representation chosen from an AdaptationSet.
#[derive(Debug)]
pub struct RepresentationInfo {
    /// `Representation@id` from the source MPD. Never empty (synthesised if
    /// the source omitted it).
    pub id: String,
    /// Declared average bandwidth in bits per second; `0` when missing.
    pub bandwidth: u64,
    /// RFC 6381 codec string, e.g. `"avc1.640028"`. Inherited from
    /// AdaptationSet when not present on the Representation. `None` both
    /// when the source MPD declares no `codecs` attribute anywhere in the
    /// chain, and when a declared value fails [`Rfc6381Codec`]'s validation
    /// floor — this field is informational only (no downstream reader
    /// currently consumes it), so a malformed manifest value degrades
    /// silently rather than failing the parse.
    pub codecs: Option<Rfc6381Codec>,
    /// MIME type, e.g. `"video/mp4"`. Inherited from AdaptationSet when not
    /// present on the Representation.
    pub mime_type: Option<String>,
    /// Declared frame width in pixels.
    pub width: Option<u32>,
    /// Declared frame height in pixels.
    pub height: Option<u32>,
    /// RFC 5646 language tag for the AdaptationSet.
    pub lang: Option<String>,
    /// Resolved BaseURL chain for this Representation:
    /// MPD BaseURL ▸ Period BaseURL ▸ AdaptationSet BaseURL ▸ Representation BaseURL.
    /// Always non-empty.
    pub base_urls: Vec<Url>,
    /// Segment plan. Populated by [`parse_mpd`]. Drives URL enumeration
    /// during download.
    pub plan: SegmentPlan,
}

/// Parse an MPD body and select the highest-bandwidth video representation
/// (and matching audio, when any) from the first period.
///
/// `base_url` is the URL the MPD itself was fetched from; it acts as the
/// root for resolving relative `<BaseURL>` elements.
///
/// # Errors
///
/// Returns [`DashError::Parse`] on malformed XML or missing `<Period>`,
/// [`DashError::DynamicMpd`] for live MPDs, [`DashError::DrmProtected`] when
/// any `ContentProtection` element is present, and
/// [`DashError::NoVideoRepresentation`] when no video AdaptationSet is found.
pub fn parse_mpd(body: &str, base_url: &Url) -> Result<ParsedManifest, DashError> {
    let mpd = dash_mpd::parse(body).map_err(|e| DashError::Parse(e.to_string()))?;

    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(DashError::DynamicMpd);
    }

    // DRM gate. Refuse before representation selection so DRM beats
    // "no video repr" in error precedence.
    if mpd_has_drm(&mpd) {
        return Err(DashError::DrmProtected);
    }

    let period_count = mpd.periods.len();
    if period_count > 1 {
        log::warn!(
            "MPD has {period_count} periods; only the first is consumed (multi-period support deferred)"
        );
    }

    let period = mpd
        .periods
        .first()
        .ok_or_else(|| DashError::Parse("no <Period>".into()))?;

    let mpd_base_urls = resolve_base_urls(&mpd.base_url, base_url);
    let period_base_urls = resolve_period_base_urls(period, &mpd_base_urls);

    let period_duration = period
        .duration
        .or(mpd.mediaPresentationDuration)
        .unwrap_or_default();

    let video = pick_video(period, &period_base_urls, period_duration)?;
    let audio = match pick_audio(period, &period_base_urls, period_duration) {
        Ok(info) => Some(info),
        Err(DashError::NoAudioRepresentation) => None,
        Err(e) => return Err(e),
    };

    Ok(ParsedManifest {
        period_duration,
        video,
        audio,
        mpd_base_urls,
        period_count,
    })
}

/// Returns true if any `ContentProtection` element appears at MPD, Period,
/// AdaptationSet, or Representation scope.
fn mpd_has_drm(mpd: &dash_mpd::MPD) -> bool {
    !mpd.ContentProtection.is_empty()
        || mpd.periods.iter().any(|period| {
            !period.ContentProtection.is_empty()
                || period.adaptations.iter().any(|aset| {
                    !aset.ContentProtection.is_empty()
                        || aset
                            .representations
                            .iter()
                            .any(|repr| !repr.ContentProtection.is_empty())
                })
        })
}

/// Resolve a list of `<BaseURL>` elements against a parent URL. When the
/// list is empty, returns `[parent]` so the chain is always non-empty.
/// Entries that fail to parse against the parent are dropped with a warning.
fn resolve_base_urls(base_urls: &[dash_mpd::BaseURL], parent: &Url) -> Vec<Url> {
    if base_urls.is_empty() {
        return vec![parent.clone()];
    }
    let mut out = Vec::with_capacity(base_urls.len());
    for b in base_urls {
        match parent.join(&b.base) {
            Ok(u) => out.push(u),
            Err(e) => log::warn!("failed to resolve BaseURL {:?}: {e}", b.base),
        }
    }
    if out.is_empty() {
        // All entries failed — fall back to parent so the chain stays non-empty.
        out.push(parent.clone());
    }
    out
}

/// Build the Period-scoped BaseURL chain by resolving Period BaseURLs
/// against the MPD-level chain (using the first MPD BaseURL as the parent).
fn resolve_period_base_urls(period: &dash_mpd::Period, mpd_base_urls: &[Url]) -> Vec<Url> {
    // resolve_base_urls always returns a non-empty Vec, so .first() should
    // not fail here; the unwrap_or_else is defensive only.
    let parent = mpd_base_urls.first().cloned().unwrap_or_else(|| {
        #[allow(clippy::unwrap_used)]
        Url::parse("about:blank").unwrap()
    });
    resolve_base_urls(&period.BaseURL, &parent)
}

/// Build the AdaptationSet-scoped BaseURL chain by resolving the AS's
/// BaseURLs against the period chain.
fn resolve_aset_base_urls(aset: &dash_mpd::AdaptationSet, period_base_urls: &[Url]) -> Vec<Url> {
    let parent = period_base_urls.first().cloned().unwrap_or_else(|| {
        #[allow(clippy::unwrap_used)]
        Url::parse("about:blank").unwrap()
    });
    resolve_base_urls(&aset.BaseURL, &parent)
}

/// Build the Representation-scoped BaseURL chain by resolving the
/// Representation's BaseURLs against the AdaptationSet chain.
fn resolve_repr_base_urls(repr: &dash_mpd::Representation, aset_base_urls: &[Url]) -> Vec<Url> {
    let parent = aset_base_urls.first().cloned().unwrap_or_else(|| {
        #[allow(clippy::unwrap_used)]
        Url::parse("about:blank").unwrap()
    });
    resolve_base_urls(&repr.BaseURL, &parent)
}

/// Returns true if the AdaptationSet describes video content. Trusts
/// `@contentType` first, falls back to `@mimeType` prefix.
fn is_video_aset(aset: &dash_mpd::AdaptationSet) -> bool {
    if let Some(ct) = aset.contentType.as_deref() {
        return ct.eq_ignore_ascii_case("video");
    }
    aset.mimeType
        .as_deref()
        .is_some_and(|m| m.to_ascii_lowercase().starts_with("video/"))
}

/// Returns true if the AdaptationSet describes audio content. Trusts
/// `@contentType` first, falls back to `@mimeType` prefix.
fn is_audio_aset(aset: &dash_mpd::AdaptationSet) -> bool {
    if let Some(ct) = aset.contentType.as_deref() {
        return ct.eq_ignore_ascii_case("audio");
    }
    aset.mimeType
        .as_deref()
        .is_some_and(|m| m.to_ascii_lowercase().starts_with("audio/"))
}

/// Pick the highest-bandwidth video Representation across all video
/// AdaptationSets in the period.
fn pick_video(
    period: &dash_mpd::Period,
    period_base_urls: &[Url],
    period_duration: Duration,
) -> Result<RepresentationInfo, DashError> {
    match pick_representation(period, period_base_urls, period_duration, is_video_aset) {
        Ok(Some(info)) => Ok(info),
        Ok(None) => Err(DashError::NoVideoRepresentation),
        Err(e) => Err(e),
    }
}

/// Pick the highest-bandwidth audio Representation across all audio
/// AdaptationSets in the period.
fn pick_audio(
    period: &dash_mpd::Period,
    period_base_urls: &[Url],
    period_duration: Duration,
) -> Result<RepresentationInfo, DashError> {
    match pick_representation(period, period_base_urls, period_duration, is_audio_aset) {
        Ok(Some(info)) => Ok(info),
        Ok(None) => Err(DashError::NoAudioRepresentation),
        Err(e) => Err(e),
    }
}

/// Generic Representation picker: filter AdaptationSets by `match_aset`,
/// then pick the highest-bandwidth Representation from the union.
///
/// Returns `Ok(None)` when no Representation matched the filter; returns
/// `Err` when a Representation was selected but its segment plan could not
/// be built (no SegmentTemplate / SegmentList found, missing required attrs).
fn pick_representation(
    period: &dash_mpd::Period,
    period_base_urls: &[Url],
    period_duration: Duration,
    match_aset: fn(&dash_mpd::AdaptationSet) -> bool,
) -> Result<Option<RepresentationInfo>, DashError> {
    let mut best: Option<(u64, &dash_mpd::AdaptationSet, &dash_mpd::Representation)> = None;

    for aset in &period.adaptations {
        if !match_aset(aset) {
            continue;
        }
        for repr in &aset.representations {
            let bw = repr.bandwidth.unwrap_or(0);
            match best {
                Some((b, _, _)) if b >= bw => {}
                _ => best = Some((bw, aset, repr)),
            }
        }
    }

    let Some((bandwidth, aset, repr)) = best else {
        return Ok(None);
    };

    let aset_base_urls = resolve_aset_base_urls(aset, period_base_urls);
    let base_urls = resolve_repr_base_urls(repr, &aset_base_urls);

    let id = repr
        .id
        .clone()
        .unwrap_or_else(|| format!("repr-{bandwidth}"));

    let codecs = repr
        .codecs
        .clone()
        .or_else(|| aset.codecs.clone())
        .and_then(|s| Rfc6381Codec::new(s).ok());
    let mime_type = repr.mimeType.clone().or_else(|| aset.mimeType.clone());

    let plan = build_segment_plan(aset, repr, period_duration, &id, bandwidth)?;

    Ok(Some(RepresentationInfo {
        id,
        bandwidth,
        codecs,
        mime_type,
        width: repr
            .width
            .or(aset.width)
            .and_then(|w| u32::try_from(w).ok()),
        height: repr
            .height
            .or(aset.height)
            .and_then(|h| u32::try_from(h).ok()),
        lang: aset.lang.clone(),
        base_urls,
        plan,
    }))
}

/// Build a `SegmentPlan` for the chosen Representation, applying DASH
/// inheritance: prefer Representation-level over AdaptationSet-level for
/// both `<SegmentTemplate>` and `<SegmentList>`.
fn build_segment_plan(
    aset: &dash_mpd::AdaptationSet,
    repr: &dash_mpd::Representation,
    period_duration: Duration,
    representation_id: &str,
    bandwidth: u64,
) -> Result<SegmentPlan, DashError> {
    if let Some(st) = repr
        .SegmentTemplate
        .as_ref()
        .or(aset.SegmentTemplate.as_ref())
    {
        return build_template_plan(st, period_duration, representation_id, bandwidth);
    }
    if let Some(sl) = repr.SegmentList.as_ref().or(aset.SegmentList.as_ref()) {
        return Ok(build_list_plan(sl));
    }
    Err(DashError::Parse(format!(
        "Representation {representation_id} has no SegmentTemplate or SegmentList"
    )))
}

fn build_template_plan(
    st: &dash_mpd::SegmentTemplate,
    period_duration: Duration,
    representation_id: &str,
    bandwidth: u64,
) -> Result<SegmentPlan, DashError> {
    let media = st.media.clone().ok_or_else(|| {
        DashError::Parse(format!(
            "Representation {representation_id} SegmentTemplate has no @media"
        ))
    })?;
    let init = st.initialization.clone();
    // Initialization byte-range: prefer the `<Initialization @range>` child
    // (ISO/IEC 23009-1 §5.3.9.4.3). `SegmentTemplate@initialization` is a
    // URL string, not a range, so the range only comes from the child.
    let init_byte_range = st
        .Initialization
        .as_ref()
        .and_then(|i| i.range.as_deref())
        .and_then(parse_byte_range);
    let start_number = st.startNumber.unwrap_or(1);
    let timescale = st.timescale.unwrap_or(1).max(1);

    if let Some(timeline) = st.SegmentTimeline.as_ref() {
        let entries: Vec<TimelineEntry> = timeline
            .segments
            .iter()
            .map(|s| TimelineEntry {
                t: s.t,
                d: s.d,
                r: s.r,
            })
            .collect();
        return Ok(SegmentPlan::Timeline(SegmentTimelinePlan {
            init,
            init_byte_range,
            media,
            start_number,
            timescale,
            entries,
            representation_id: representation_id.to_string(),
            bandwidth,
        }));
    }

    // Plain SegmentTemplate@duration mode.
    let seg_dur_f64 = st.duration.ok_or_else(|| {
        DashError::Parse(format!(
            "Representation {representation_id} SegmentTemplate has no @duration and no SegmentTimeline"
        ))
    })?;
    if !seg_dur_f64.is_finite() || seg_dur_f64 <= 0.0 {
        return Err(DashError::Parse(format!(
            "Representation {representation_id} SegmentTemplate @duration is non-positive: {seg_dur_f64}"
        )));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let segment_duration_ts = seg_dur_f64.round() as u64;
    let segment_duration_ts = segment_duration_ts.max(1);

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let total_segments = {
        let period_secs = period_duration.as_secs_f64();
        let total_ts = period_secs * timescale as f64;
        (total_ts / seg_dur_f64).ceil() as u64
    };

    Ok(SegmentPlan::Template(SegmentTemplatePlan {
        init,
        init_byte_range,
        media,
        start_number,
        timescale,
        segment_duration_ts,
        total_segments,
        representation_id: representation_id.to_string(),
        bandwidth,
    }))
}

fn build_list_plan(sl: &dash_mpd::SegmentList) -> SegmentPlan {
    let init = sl.Initialization.as_ref().and_then(|i| i.sourceURL.clone());
    let init_byte_range = sl
        .Initialization
        .as_ref()
        .and_then(|i| i.range.as_deref())
        .and_then(parse_byte_range);
    let urls = sl
        .segment_urls
        .iter()
        .filter_map(|u| u.media.clone())
        .collect();
    SegmentPlan::List(SegmentListPlan {
        init,
        init_byte_range,
        urls,
    })
}

/// Parse a DASH `<Initialization @range>` byte-range attribute into an
/// `(start, end_exclusive)` tuple.
///
/// Per ISO/IEC 23009-1 §5.3.9.4.3 the attribute uses RFC 7233 `bytes-range-spec`
/// form (`"start-end"`, inclusive). Returned tuple is `(start, end + 1)` to
/// match rdlp's exclusive-end convention (mirrors `Fragment.byte_range`).
///
/// Returns `None` on any malformed input: empty string, non-numeric components,
/// missing dash, reversed start/end, or arithmetic overflow on `end + 1`.
#[must_use]
pub(crate) fn parse_byte_range(s: &str) -> Option<(u64, u64)> {
    let (start_s, end_s) = s.split_once('-')?;
    if start_s.is_empty() || end_s.is_empty() {
        return None;
    }
    let start: u64 = start_s.parse().ok()?;
    let end: u64 = end_s.parse().ok()?;
    if end < start {
        return None;
    }
    let end_exclusive = end.checked_add(1)?;
    Some((start, end_exclusive))
}

#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn parses_zero_based_range() {
        assert_eq!(parse_byte_range("0-739"), Some((0, 740)));
    }

    #[test]
    fn parses_single_byte_range() {
        assert_eq!(parse_byte_range("100-100"), Some((100, 101)));
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(parse_byte_range(""), None);
    }

    #[test]
    fn rejects_non_numeric() {
        assert_eq!(parse_byte_range("abc"), None);
        assert_eq!(parse_byte_range("0-abc"), None);
    }

    #[test]
    fn rejects_missing_component() {
        assert_eq!(parse_byte_range("0-"), None);
        assert_eq!(parse_byte_range("-5"), None);
    }

    #[test]
    fn rejects_reversed() {
        assert_eq!(parse_byte_range("5-3"), None);
    }

    #[test]
    fn rejects_overflow_on_end_plus_one() {
        assert_eq!(parse_byte_range(&format!("0-{}", u64::MAX)), None);
    }

    #[test]
    fn rejects_whitespace_in_components() {
        // RFC 7233 grammar disallows interior whitespace; Rust's u64 parser
        // also rejects leading/trailing whitespace. Lock the contract so a
        // future refactor doesn't silently add `trim()` and widen acceptance.
        assert_eq!(parse_byte_range(" 0-739"), None);
        assert_eq!(parse_byte_range("0-739 "), None);
        assert_eq!(parse_byte_range("0 - 739"), None);
    }

    fn segment_list_mpd_with_init(init_attrs: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S">
  <Period duration="PT10S">
    <AdaptationSet contentType="video" mimeType="video/mp4">
      <Representation id="v1" bandwidth="1000000">
        <BaseURL>https://example.com/video.mp4</BaseURL>
        <SegmentList timescale="1000" duration="5000">
          <Initialization {init_attrs}/>
          <SegmentURL media="seg1.m4s"/>
          <SegmentURL media="seg2.m4s"/>
        </SegmentList>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#
        )
    }

    #[test]
    fn list_plan_range_only_no_source_url() {
        let xml = segment_list_mpd_with_init(r#"range="0-739""#);
        let base = Url::parse("https://example.com/manifest.mpd").unwrap();
        let parsed = parse_mpd(&xml, &base).expect("parse");
        match &parsed.video.plan {
            SegmentPlan::List(p) => {
                assert_eq!(p.init, None, "no sourceURL → init field stays None");
                assert_eq!(p.init_byte_range, Some((0, 740)));
            }
            other => panic!("expected SegmentPlan::List, got {other:?}"),
        }
    }

    #[test]
    fn list_plan_source_url_with_range() {
        let xml = segment_list_mpd_with_init(r#"sourceURL="init.m4s" range="0-739""#);
        let base = Url::parse("https://example.com/manifest.mpd").unwrap();
        let parsed = parse_mpd(&xml, &base).expect("parse");
        match &parsed.video.plan {
            SegmentPlan::List(p) => {
                assert_eq!(p.init.as_deref(), Some("init.m4s"));
                assert_eq!(p.init_byte_range, Some((0, 740)));
            }
            other => panic!("expected SegmentPlan::List, got {other:?}"),
        }
    }

    #[test]
    fn list_plan_no_init_attrs_at_all() {
        let xml = segment_list_mpd_with_init("");
        let base = Url::parse("https://example.com/manifest.mpd").unwrap();
        let parsed = parse_mpd(&xml, &base).expect("parse");
        match &parsed.video.plan {
            SegmentPlan::List(p) => {
                assert_eq!(p.init, None);
                assert_eq!(p.init_byte_range, None);
            }
            other => panic!("expected SegmentPlan::List, got {other:?}"),
        }
    }

    #[test]
    fn init_url_falls_back_to_base_when_range_only() {
        let xml = segment_list_mpd_with_init(r#"range="0-739""#);
        let base = Url::parse("https://example.com/manifest.mpd").unwrap();
        let parsed = parse_mpd(&xml, &base).expect("parse");
        // When init field is None but init_byte_range is Some, the URL to
        // fetch the init segment is the first BaseURL of the Representation.
        let init = parsed.video.plan.init_url(&parsed.video.base_urls);
        assert_eq!(
            init.as_ref().map(Url::as_str),
            Some("https://example.com/video.mp4")
        );
    }
}
