//! Segment-URL resolution for the three DASH listing modes.
//!
//! `SegmentPlan` encodes the durable subset of a `<SegmentTemplate>`,
//! `<SegmentList>`, or `<SegmentTimeline>` so we can enumerate segment URLs
//! without holding onto the parsed dash-mpd tree.

#![allow(clippy::doc_markdown)]

use std::time::Duration;
use url::Url;

/// One of the three DASH segment-listing modes.
#[derive(Debug, Clone)]
pub enum SegmentPlan {
    /// `<SegmentTemplate>` with `@duration` (no SegmentTimeline child).
    Template(SegmentTemplatePlan),
    /// `<SegmentTemplate>` containing a `<SegmentTimeline>`.
    Timeline(SegmentTimelinePlan),
    /// Explicit `<SegmentList>` with per-segment URLs.
    List(SegmentListPlan),
}

/// `<SegmentTemplate>` with `@duration` (no SegmentTimeline child).
#[derive(Debug, Clone)]
pub struct SegmentTemplatePlan {
    /// Initialisation segment URL template (relative to the BaseURL chain).
    pub init: Option<String>,
    /// Optional byte-range on the initialisation segment, as
    /// `(start, end_exclusive)` — populated from
    /// `<Initialization @range="start-end">` (ISO/IEC 23009-1 §5.3.9.4.3).
    /// When `init` is `None` and this is `Some`, the init segment is fetched
    /// from the first BaseURL of the Representation.
    pub init_byte_range: Option<(u64, u64)>,
    /// Media template string e.g. `"video/seg-$Number$.m4s"`.
    pub media: String,
    /// First segment number (DASH `@startNumber`, defaults to 1).
    pub start_number: u64,
    /// Timescale for `@duration` and `$Time$` substitutions.
    pub timescale: u64,
    /// Default segment duration in `timescale` units.
    pub segment_duration_ts: u64,
    /// Computed total segment count: `ceil(period_duration_ts / segment_duration_ts)`.
    pub total_segments: u64,
    /// `Representation@id` used for `$RepresentationID$` substitution.
    pub representation_id: String,
    /// `Representation@bandwidth` used for `$Bandwidth$` substitution.
    pub bandwidth: u64,
}

/// `<SegmentTemplate>` containing a `<SegmentTimeline>`.
#[derive(Debug, Clone)]
pub struct SegmentTimelinePlan {
    /// Initialisation segment URL template.
    pub init: Option<String>,
    /// Optional byte-range on the initialisation segment — see
    /// [`SegmentTemplatePlan::init_byte_range`] for semantics.
    pub init_byte_range: Option<(u64, u64)>,
    /// Media template string.
    pub media: String,
    /// First segment number.
    pub start_number: u64,
    /// Timescale for `$Time$` substitutions.
    pub timescale: u64,
    /// Ordered list of `<S>` entries describing segment durations.
    pub entries: Vec<TimelineEntry>,
    /// `Representation@id`.
    pub representation_id: String,
    /// `Representation@bandwidth`.
    pub bandwidth: u64,
}

/// One `<S>` inside a `<SegmentTimeline>`.
#[derive(Debug, Clone)]
pub struct TimelineEntry {
    /// Optional explicit start time (`@t`). When `None`, the entry continues
    /// from the previous entry's end.
    pub t: Option<u64>,
    /// Required `@d` — segment duration in timescale units.
    pub d: u64,
    /// `@r` — None or 0 means "no repeat" (emit one segment), positive means
    /// "emit 1 + r segments", negative means "repeat to end of period".
    pub r: Option<i64>,
}

/// `<SegmentList>` — explicit per-segment URL listing.
#[derive(Debug, Clone)]
pub struct SegmentListPlan {
    /// Initialisation segment URL (relative).
    pub init: Option<String>,
    /// Optional byte-range on the initialisation segment — see
    /// [`SegmentTemplatePlan::init_byte_range`] for semantics.
    pub init_byte_range: Option<(u64, u64)>,
    /// Per-segment relative URLs in playback order.
    pub urls: Vec<String>,
}

impl SegmentPlan {
    /// Resolve the initialisation segment URL against the supplied BaseURL chain.
    ///
    /// Returns `None` when no init is declared (neither `sourceURL` nor a
    /// byte-range). When the plan carries only a byte-range
    /// (`<Initialization range="…"/>` without `@sourceURL` — ISO/IEC 23009-1
    /// §5.3.9.4.3), the init URL falls back to the first BaseURL of the
    /// chain and the caller is expected to issue a ranged GET using
    /// [`Self::init_byte_range`].
    #[must_use]
    pub fn init_url(&self, base: &[Url]) -> Option<Url> {
        let (init, range) = match self {
            Self::Template(t) => (t.init.as_deref(), t.init_byte_range),
            Self::Timeline(t) => (t.init.as_deref(), t.init_byte_range),
            Self::List(t) => (t.init.as_deref(), t.init_byte_range),
        };
        match (init, range) {
            (Some(rel), _) => join(base, rel).ok(),
            (None, Some(_)) => base.first().cloned(),
            (None, None) => None,
        }
    }

    /// The byte-range, if any, attached to the initialisation segment.
    ///
    /// `Some((start, end_exclusive))` matches the convention used by
    /// `Fragment.byte_range` / `Fragment.init_byte_range` in `rdlp-types`.
    #[must_use]
    pub const fn init_byte_range(&self) -> Option<(u64, u64)> {
        match self {
            Self::Template(t) => t.init_byte_range,
            Self::Timeline(t) => t.init_byte_range,
            Self::List(t) => t.init_byte_range,
        }
    }

    /// Enumerate every media-segment URL.
    ///
    /// `period_duration_ts` is only consulted by the Timeline branch (to
    /// expand entries with negative `r`); Template and List ignore it.
    #[must_use]
    pub fn segment_urls(&self, base: &[Url], period_duration_ts: u64) -> Vec<Url> {
        match self {
            Self::Template(t) => template_urls(t, base),
            Self::Timeline(t) => timeline_urls(t, base, period_duration_ts),
            Self::List(t) => t.urls.iter().filter_map(|u| join(base, u).ok()).collect(),
        }
    }

    /// Total media duration covered by this plan, derived from segment counts.
    /// Useful for diagnostic logging — the canonical period duration comes
    /// from the manifest.
    #[must_use]
    pub fn total_duration(&self) -> Duration {
        match self {
            Self::Template(t) => Duration::from_secs(
                t.total_segments.saturating_mul(t.segment_duration_ts) / t.timescale.max(1),
            ),
            Self::Timeline(t) => {
                let total_ts: u64 = t.entries.iter().map(timeline_entry_ts).sum();
                Duration::from_secs(total_ts / t.timescale.max(1))
            }
            Self::List(_) => Duration::from_secs(0),
        }
    }
}

fn timeline_entry_ts(entry: &TimelineEntry) -> u64 {
    let repeats = match entry.r {
        None | Some(0) => 0u64,
        Some(k) if k > 0 => u64::try_from(k).unwrap_or(0),
        Some(_neg) => 0, // diagnostic only — true value depends on period duration
    };
    entry.d.saturating_mul(repeats.saturating_add(1))
}

fn template_urls(t: &SegmentTemplatePlan, base: &[Url]) -> Vec<Url> {
    (0..t.total_segments)
        .filter_map(|i| {
            let n = t.start_number + i;
            let raw = substitute(
                &t.media,
                &t.representation_id,
                &DashTemplateVars {
                    number: n,
                    time: n.saturating_mul(t.segment_duration_ts),
                    bandwidth: t.bandwidth,
                },
            );
            join(base, &raw).ok()
        })
        .collect()
}

fn timeline_urls(t: &SegmentTimelinePlan, base: &[Url], period_duration_ts: u64) -> Vec<Url> {
    let mut out = Vec::new();
    let mut n = t.start_number;
    let mut ts: u64 = 0;
    for entry in &t.entries {
        if let Some(explicit_t) = entry.t {
            ts = explicit_t;
        }
        // DASH SegmentTimeline @r:
        //   None or Some(0) → emit one segment (no repeats)
        //   Some(k>0) → emit 1 + k segments
        //   Some(neg) → repeat until end of period
        let repeats: u64 = match entry.r {
            None | Some(0) => 0,
            Some(k) if k > 0 => u64::try_from(k).unwrap_or(0),
            Some(_neg) => {
                let remaining = period_duration_ts.saturating_sub(ts);
                remaining
                    .checked_div(entry.d)
                    .map_or(0, |q| q.saturating_sub(1))
            }
        };
        for _ in 0..=repeats {
            let raw = substitute(
                &t.media,
                &t.representation_id,
                &DashTemplateVars {
                    number: n,
                    time: ts,
                    bandwidth: t.bandwidth,
                },
            );
            if let Ok(u) = join(base, &raw) {
                out.push(u);
            }
            n += 1;
            ts = ts.saturating_add(entry.d);
        }
    }
    out
}

/// Named variables for DASH `$SegmentTemplate$` substitution.
///
/// Replaces the previous 3-positional-`u64` signature so the compiler catches
/// argument-swap bugs at the call site.
#[derive(Debug, Clone, Copy)]
pub(super) struct DashTemplateVars {
    pub number: u64,
    pub time: u64,
    pub bandwidth: u64,
}

fn substitute(template: &str, rep_id: &str, vars: &DashTemplateVars) -> String {
    template
        .replace("$RepresentationID$", rep_id)
        .replace("$Number$", &vars.number.to_string())
        .replace("$Time$", &vars.time.to_string())
        .replace("$Bandwidth$", &vars.bandwidth.to_string())
}

/// Resolve `rel` against the supplied BaseURL chain. The chain is walked in
/// order: each successive entry resolves against the previous resolution.
/// The chain is always non-empty (manifest::resolve_base_urls guarantees this).
fn join(base: &[Url], rel: &str) -> Result<Url, url::ParseError> {
    let mut current = base
        .first()
        .cloned()
        .ok_or(url::ParseError::RelativeUrlWithoutBase)?;
    for b in base.iter().skip(1) {
        current = current.join(b.as_str())?;
    }
    current.join(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_uses_named_vars() {
        let vars = DashTemplateVars {
            number: 42,
            time: 1000,
            bandwidth: 500_000,
        };
        let result = substitute(
            "seg_$RepresentationID$_$Number$_$Time$_$Bandwidth$.m4s",
            "video_720p",
            &vars,
        );
        assert_eq!(result, "seg_video_720p_42_1000_500000.m4s");
    }

    #[test]
    fn substitute_missing_template_var_left_untouched() {
        let vars = DashTemplateVars {
            number: 1,
            time: 0,
            bandwidth: 0,
        };
        let result = substitute("$Unknown$_$Number$", "rep", &vars);
        assert!(result.contains("$Unknown$"));
        assert!(result.contains('1'));
    }
}
