//! Format selection evaluation logic.
//!
//! Evaluates parsed format expressions against a list of available
//! [`Format`]s, selecting the best matching formats.

use super::{BaseSelector, Filter, FilterField, FilterOp, FilterValue, FormatSpec, Selector};
use crate::format::Format;

/// Evaluate a `FormatSpec` against the format list.
pub(super) fn select_spec<'a>(spec: &FormatSpec, formats: &'a [Format]) -> Vec<&'a Format> {
    match spec {
        FormatSpec::Single(sel) => select_one(sel, formats).into_iter().collect(),
        FormatSpec::Merge { video, audio } => {
            let v = select_one(video, formats);
            let a = select_one(audio, formats);
            v.into_iter().chain(a).collect()
        }
    }
}

/// Select a single format matching a `Selector`.
fn select_one<'a>(sel: &Selector, formats: &'a [Format]) -> Option<&'a Format> {
    let candidates = formats
        .iter()
        .filter(|f| !f.has_drm.unwrap_or(false))
        .filter(|f| matches_base(&sel.base, f))
        .filter(|f| sel.filters.iter().all(|filter| matches_filter(filter, f)));

    match sort_direction(&sel.base) {
        SortDirection::Best => candidates.max_by(|a, b| rank_formats(&sel.base, a, b)),
        SortDirection::Worst => candidates.min_by(|a, b| rank_formats(&sel.base, a, b)),
    }
}

/// Whether a format is a candidate for the given base selector.
///
/// For `Best`/`Worst`, a format qualifies if it has both video and audio,
/// OR if codecs are unknown (both `None`) -- assumed to be a combined stream.
fn matches_base(base: &BaseSelector, f: &Format) -> bool {
    let codecs_unknown = f.vcodec.is_none() && f.acodec.is_none();
    match base {
        BaseSelector::Best | BaseSelector::Worst => {
            (f.has_video() && f.has_audio()) || codecs_unknown
        }
        BaseSelector::BestVideo | BaseSelector::WorstVideo => f.has_video() && !f.has_audio(),
        BaseSelector::BestVideoStar => f.has_video() || codecs_unknown,
        BaseSelector::BestAudio | BaseSelector::WorstAudio => f.has_audio() && !f.has_video(),
        BaseSelector::BestAudioStar => f.has_audio() || codecs_unknown,
        BaseSelector::FormatId(id) => f.format_id == *id,
    }
}

/// Whether a format passes a single filter condition.
fn matches_filter(filter: &Filter, f: &Format) -> bool {
    match &filter.field {
        FilterField::Height => {
            compare_opt_num(f.height.map(|v| v as f64), &filter.op, &filter.value)
        }
        FilterField::Width => compare_opt_num(f.width.map(|v| v as f64), &filter.op, &filter.value),
        FilterField::Fps => compare_opt_num(f.fps, &filter.op, &filter.value),
        FilterField::Tbr => compare_opt_num(f.tbr, &filter.op, &filter.value),
        FilterField::Vbr => compare_opt_num(f.vbr, &filter.op, &filter.value),
        FilterField::Abr => compare_opt_num(f.abr, &filter.op, &filter.value),
        FilterField::Asr => compare_opt_num(f.asr.map(|v| v as f64), &filter.op, &filter.value),
        FilterField::Filesize => {
            compare_opt_num(f.filesize.map(|v| v as f64), &filter.op, &filter.value)
        }
        FilterField::Ext => compare_str(&f.ext, &filter.op, &filter.value),
        FilterField::Vcodec => match &f.vcodec {
            Some(v) => compare_str(v, &filter.op, &filter.value),
            None => false,
        },
        FilterField::Acodec => match &f.acodec {
            Some(v) => compare_str(v, &filter.op, &filter.value),
            None => false,
        },
        FilterField::Protocol => compare_str(f.protocol.as_str(), &filter.op, &filter.value),
        FilterField::FormatId => compare_str(&f.format_id, &filter.op, &filter.value),
    }
}

/// Compare an optional numeric value against a filter.
/// Returns `false` if the value is `None` (conservative -- missing data doesn't match).
fn compare_opt_num(val: Option<f64>, op: &FilterOp, filter_val: &FilterValue) -> bool {
    let Some(val) = val else {
        return false;
    };
    let target = match filter_val {
        FilterValue::Number(n) => *n,
        FilterValue::Text(s) => {
            let Ok(n) = s.parse::<f64>() else {
                return false;
            };
            n
        }
    };
    match op {
        FilterOp::Eq => (val - target).abs() < f64::EPSILON,
        FilterOp::Ne => (val - target).abs() >= f64::EPSILON,
        FilterOp::Lt => val < target,
        FilterOp::Le => val <= target,
        FilterOp::Gt => val > target,
        FilterOp::Ge => val >= target,
    }
}

/// Compare a string value against a filter.
fn compare_str(val: &str, op: &FilterOp, filter_val: &FilterValue) -> bool {
    let target = match filter_val {
        FilterValue::Text(s) => s.as_str(),
        FilterValue::Number(n) => {
            // Numeric filter on string field -- convert number to string for comparison
            let s = n.to_string();
            return match op {
                FilterOp::Eq => val == s,
                FilterOp::Ne => val != s,
                _ => false, // Ordering ops don't make sense for string-vs-number
            };
        }
    };
    match op {
        FilterOp::Eq => val == target,
        FilterOp::Ne => val != target,
        // String ordering (lexicographic) for <, <=, >, >=
        FilterOp::Lt => val < target,
        FilterOp::Le => val <= target,
        FilterOp::Gt => val > target,
        FilterOp::Ge => val >= target,
    }
}

#[derive(Clone, Copy)]
enum SortDirection {
    Best,
    Worst,
}

fn sort_direction(base: &BaseSelector) -> SortDirection {
    match base {
        BaseSelector::Worst | BaseSelector::WorstVideo | BaseSelector::WorstAudio => {
            SortDirection::Worst
        }
        _ => SortDirection::Best,
    }
}

/// Rank two formats by quality. Used with `max_by` (best) or `min_by` (worst).
fn rank_formats(base: &BaseSelector, a: &Format, b: &Format) -> std::cmp::Ordering {
    match base {
        BaseSelector::BestAudio | BaseSelector::BestAudioStar | BaseSelector::WorstAudio => {
            // Audio ranking: abr > asr
            cmp_opt_f64(a.abr, b.abr).then(a.asr.cmp(&b.asr))
        }
        BaseSelector::BestVideo | BaseSelector::BestVideoStar | BaseSelector::WorstVideo => {
            // Video ranking: height > vbr > fps
            a.height
                .cmp(&b.height)
                .then(cmp_opt_f64(a.vbr, b.vbr))
                .then(cmp_opt_f64(a.fps, b.fps))
        }
        _ => {
            // Combined/general ranking: quality > height > tbr > fps
            a.quality
                .cmp(&b.quality)
                .then(a.height.cmp(&b.height))
                .then(cmp_opt_f64(a.tbr, b.tbr))
                .then(cmp_opt_f64(a.fps, b.fps))
        }
    }
}

/// Compare two `Option<f64>` values for ranking purposes.
///
/// When both sides have values, compare them numerically.
/// When either side is `None`, treat as `Equal` -- missing data should not
/// bias the ranking (prevents `Some(x) > None` from `Option::partial_cmp`
/// causing HLS formats with known bitrate to always outrank direct downloads
/// that lack bitrate metadata).
fn cmp_opt_f64(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
        _ => std::cmp::Ordering::Equal,
    }
}
