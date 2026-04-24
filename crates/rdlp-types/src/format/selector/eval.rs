//! Format selection evaluation logic.
//!
//! Evaluates parsed format expressions against a list of available
//! [`Format`]s, selecting the best matching formats.

use super::{
    Filter, FilterField, FilterOp, FilterValue, FormatSpec, FormatToken, Quality, Selector,
    SelectorNode, StreamType, eval_node, sort::FormatSorter,
};
use crate::format::Format;
use regex::Regex;

/// Evaluate a `FormatSpec` against the format list.
pub(super) fn select_spec<'a>(spec: &FormatSpec, formats: &'a [Format]) -> Vec<&'a Format> {
    match spec {
        FormatSpec::Single(sel) => {
            // `all` and `mergeall` produce multi-format results; handle before
            // the scalar `select_one` path.
            match &sel.base {
                FormatToken::All => return select_all_formats(formats),
                FormatToken::MergeAll => return select_mergeall_formats(formats),
                FormatToken::Group(inner_nodes) => {
                    return select_group(inner_nodes, &sel.filters, formats);
                }
                _ => {}
            }
            select_one(sel, formats).into_iter().collect()
        }
        FormatSpec::Merge { video, audio } => {
            // A merge requires BOTH sides. If either side selects nothing,
            // the whole merge fails (empty result), and the outer fallback
            // chain (`.../best`) takes over. This mirrors yt-dlp's default
            // `bestvideo*+bestaudio/best` semantics: on muxed-only sites the
            // merge fails cleanly and the scalar `best` branch wins.
            match (select_one(video, formats), select_one(audio, formats)) {
                (Some(v), Some(a)) => vec![v, a],
                _ => Vec::new(),
            }
        }
        FormatSpec::Group { inner, filters } => {
            // This variant is currently unused by the parser (groups are represented
            // as FormatSpec::Single with FormatToken::Group), but kept for completeness.
            select_group(inner, filters, formats)
        }
    }
}

/// Evaluate `all` — return every non-DRM format sorted best first.
///
/// Ranking uses the yt-dlp default sort order.
fn select_all_formats<'a>(formats: &'a [Format]) -> Vec<&'a Format> {
    let mut result: Vec<&'a Format> = formats
        .iter()
        .filter(|f| !f.has_drm.unwrap_or(false))
        .collect();
    FormatSorter::default_order().sort(&mut result);
    result
}

/// Evaluate `mergeall` — return all non-DRM formats that have video or audio,
/// sorted worst first (the pipeline merges them in this order).
fn select_mergeall_formats<'a>(formats: &'a [Format]) -> Vec<&'a Format> {
    let mut result: Vec<&'a Format> = formats
        .iter()
        .filter(|f| !f.has_drm.unwrap_or(false))
        .filter(|f| f.has_video() || f.has_audio())
        .collect();
    // Sort worst first (ascending rank) so the pipeline can build up from the
    // lowest-quality stream when merging.
    let sorter = FormatSorter::default_order();
    result.sort_by(|a, b| sorter.compare(a, b));
    result
}

/// Evaluate a parenthesised group spec.
///
/// Tries each inner `SelectorNode` in order (comma-separated nodes inside
/// the parens) and returns the result of the first non-empty match.
///
/// Outer `filters` are applied to every format in the result.
fn select_group<'a>(
    inner: &[SelectorNode],
    filters: &[Filter],
    formats: &'a [Format],
) -> Vec<&'a Format> {
    for node in inner {
        let result = eval_node(node, formats);
        if !result.is_empty() {
            // Apply outer filters (if any) to each format in the group result.
            if filters.is_empty() {
                return result;
            }
            let filtered: Vec<&'a Format> = result
                .into_iter()
                .filter(|f| filters.iter().all(|filter| matches_filter(filter, f)))
                .collect();
            if !filtered.is_empty() {
                return filtered;
            }
        }
    }
    Vec::new()
}

/// Select a single format matching a `Selector`.
fn select_one<'a>(sel: &Selector, formats: &'a [Format]) -> Option<&'a Format> {
    // `All`, `MergeAll`, and `Group` are not meaningful as "pick one" selectors;
    // these are handled upstream in `select_spec`.
    match &sel.base {
        FormatToken::All | FormatToken::MergeAll | FormatToken::Group(_) => return None,
        _ => {}
    }

    let base_candidates: Vec<&'a Format> = formats
        .iter()
        .filter(|f| !f.has_drm.unwrap_or(false))
        .filter(|f| matches_token(&sel.base, f))
        .filter(|f| sel.filters.iter().all(|filter| matches_filter(filter, f)))
        .collect();

    // For `best` (StreamType::Any, Quality::Best without modified), if the
    // format list contains NO muxed formats at all (i.e. the site only provides
    // separate video-only and audio-only streams), fall back to including
    // video-only and audio-only formats.  This matches yt-dlp's behaviour for
    // such sites.  The fallback only triggers when the entire format list lacks
    // combined streams — NOT when filters happen to exclude all combined formats.
    let any_combined = formats
        .iter()
        .any(|f| !f.has_drm.unwrap_or(false) && f.has_video() && f.has_audio());
    let candidates: Vec<&'a Format> =
        if is_best_combined(&sel.base) && !any_combined && base_candidates.is_empty() {
            // Re-apply filters, but now allow video-only or audio-only formats.
            formats
                .iter()
                .filter(|f| !f.has_drm.unwrap_or(false))
                .filter(|f| f.has_video() || f.has_audio())
                .filter(|f| sel.filters.iter().all(|filter| matches_filter(filter, f)))
                .collect()
        } else {
            base_candidates
        };

    // Apply `.N` nth-best selection if specified.
    let nth = nth_of_token(&sel.base);

    match (sort_direction(&sel.base), nth) {
        (SortDirection::Best, None) => candidates
            .into_iter()
            .max_by(|a, b| rank_formats(&sel.base, a, b)),
        (SortDirection::Worst, None) => candidates
            .into_iter()
            .min_by(|a, b| rank_formats(&sel.base, a, b)),
        (SortDirection::Best, Some(n)) => {
            // Collect all matching candidates sorted best-first, then pick index n-1 (1-based).
            let mut all = candidates;
            all.sort_by(|a, b| rank_formats(&sel.base, b, a)); // best first = reverse of rank
            all.into_iter().nth((n as usize).saturating_sub(1))
        }
        (SortDirection::Worst, Some(n)) => {
            // Collect worst-first, pick index n-1.
            let mut all = candidates;
            all.sort_by(|a, b| rank_formats(&sel.base, a, b)); // worst first
            all.into_iter().nth((n as usize).saturating_sub(1))
        }
    }
}

/// Returns `true` when the token is `best` / `b` — the combined "any" best selector —
/// and not the star-modified variant.  Used for the incomplete-formats fallback.
fn is_best_combined(token: &FormatToken) -> bool {
    matches!(
        token,
        FormatToken::Keyword {
            quality: Quality::Best,
            stream_type: StreamType::Any,
            modified: false,
            ..
        }
    )
}

/// Extract the `.N` nth suffix from a `FormatToken::Keyword`, if present.
fn nth_of_token(token: &FormatToken) -> Option<u32> {
    match token {
        FormatToken::Keyword { nth, .. } => *nth,
        _ => None,
    }
}

/// Whether a format is a candidate for the given `FormatToken`.
///
/// `Extension` shorthands resolve to `best[ext=<ext>]` semantics, i.e. any
/// format with that extension.
fn matches_token(token: &FormatToken, f: &Format) -> bool {
    let codecs_unknown = f.vcodec.is_none() && f.acodec.is_none();
    match token {
        FormatToken::Keyword {
            stream_type,
            modified,
            ..
        } => match stream_type {
            StreamType::Any => {
                if *modified {
                    // `b*` / `best*` — any format with video or audio
                    f.has_video() || f.has_audio() || codecs_unknown
                } else {
                    // `b` / `best` — must have both video and audio
                    (f.has_video() && f.has_audio()) || codecs_unknown
                }
            }
            StreamType::Video => {
                if *modified {
                    // `bv*` — has video (may also have audio)
                    f.has_video() || codecs_unknown
                } else {
                    // `bv` / `bestvideo` — video-only
                    f.has_video() && !f.has_audio()
                }
            }
            StreamType::Audio => {
                if *modified {
                    // `ba*` — has audio (may also have video)
                    f.has_audio() || codecs_unknown
                } else {
                    // `ba` / `bestaudio` — audio-only
                    f.has_audio() && !f.has_video()
                }
            }
        },
        FormatToken::FormatId(id) => f.format_id == *id,
        FormatToken::Extension(ext) => f.ext == *ext,
        // All / MergeAll / Group handled before this function is called.
        FormatToken::All | FormatToken::MergeAll | FormatToken::Group(_) => true,
    }
}

/// Whether a format passes a single filter condition.
///
/// When `filter.negated` is `true`, the result of the core evaluation is inverted.
/// When `filter.non_fatal` is `true` and the field is absent on the format, the
/// format is passed through (returns `true`) rather than excluded.
fn matches_filter(filter: &Filter, f: &Format) -> bool {
    match evaluate_filter(filter, f) {
        FilterResult::Pass => !filter.negated,
        FilterResult::Fail => filter.negated,
        FilterResult::FieldMissing => {
            // Non-fatal: absent field is treated as a pass (include the format).
            // Fatal (default): absent field excludes the format.
            filter.non_fatal
        }
    }
}

/// Result of evaluating a single filter against a format.
enum FilterResult {
    /// The filter condition is satisfied.
    Pass,
    /// The filter condition is not satisfied.
    Fail,
    /// The required field is absent on the format (Option::None / unknown).
    FieldMissing,
}

/// Core filter evaluation (without negation applied).
///
/// Returns `FilterResult::FieldMissing` when the field value is absent so the
/// caller can honour the `non_fatal` flag independently of the filter outcome.
fn evaluate_filter(filter: &Filter, f: &Format) -> FilterResult {
    match &filter.field {
        FilterField::Height => {
            compare_opt_num_r(f.height.map(|v| v as f64), &filter.op, &filter.value)
        }
        FilterField::Width => {
            compare_opt_num_r(f.width.map(|v| v as f64), &filter.op, &filter.value)
        }
        FilterField::Fps => compare_opt_num_r(f.fps, &filter.op, &filter.value),
        FilterField::Tbr => compare_opt_num_r(f.tbr, &filter.op, &filter.value),
        FilterField::Vbr => compare_opt_num_r(f.vbr, &filter.op, &filter.value),
        FilterField::Abr => compare_opt_num_r(f.abr, &filter.op, &filter.value),
        FilterField::Asr => compare_opt_num_r(f.asr.map(|v| v as f64), &filter.op, &filter.value),
        FilterField::Filesize => {
            compare_opt_num_r(f.filesize.map(|v| v as f64), &filter.op, &filter.value)
        }
        FilterField::Ext => bool_to_result(compare_str(&f.ext, &filter.op, &filter.value)),
        FilterField::Vcodec => match &f.vcodec {
            Some(v) => bool_to_result(compare_str(v, &filter.op, &filter.value)),
            None => FilterResult::FieldMissing,
        },
        FilterField::Acodec => match &f.acodec {
            Some(v) => bool_to_result(compare_str(v, &filter.op, &filter.value)),
            None => FilterResult::FieldMissing,
        },
        FilterField::Protocol => {
            bool_to_result(compare_str(f.protocol.as_str(), &filter.op, &filter.value))
        }
        FilterField::FormatId => {
            bool_to_result(compare_str(&f.format_id, &filter.op, &filter.value))
        }
        // Arbitrary field names not wired to Format fields — conservatively
        // report as missing (we cannot confirm the field exists or matches).
        FilterField::Other(_) => FilterResult::FieldMissing,
    }
}

fn bool_to_result(b: bool) -> FilterResult {
    if b {
        FilterResult::Pass
    } else {
        FilterResult::Fail
    }
}

/// Compare an optional numeric value against a filter, returning a `FilterResult`.
///
/// Returns `FilterResult::FieldMissing` when `val` is `None` so the caller can
/// honour the non-fatal flag.  Returns `FilterResult::Fail` for unsupported
/// operator/value combinations (e.g. string ops on numeric fields).
fn compare_opt_num_r(val: Option<f64>, op: &FilterOp, filter_val: &FilterValue) -> FilterResult {
    let Some(val) = val else {
        return FilterResult::FieldMissing;
    };
    let target = match filter_val {
        FilterValue::Number(n) => *n,
        FilterValue::Size(bytes) => *bytes as f64,
        FilterValue::Text(s) => {
            let Ok(n) = s.parse::<f64>() else {
                return FilterResult::Fail;
            };
            n
        }
    };
    let matched = match op {
        FilterOp::Eq => (val - target).abs() < f64::EPSILON,
        FilterOp::Ne => (val - target).abs() >= f64::EPSILON,
        FilterOp::Lt => val < target,
        FilterOp::Le => val <= target,
        FilterOp::Gt => val > target,
        FilterOp::Ge => val >= target,
        // String ops on numeric fields are not meaningful.
        FilterOp::StartsWith | FilterOp::EndsWith | FilterOp::Contains | FilterOp::Regex => {
            return FilterResult::Fail;
        }
    };
    bool_to_result(matched)
}

/// Compare a string value against a filter.
fn compare_str(val: &str, op: &FilterOp, filter_val: &FilterValue) -> bool {
    match op {
        FilterOp::StartsWith | FilterOp::EndsWith | FilterOp::Contains | FilterOp::Regex => {
            // For string ops, coerce the filter value to a string representation.
            // A bare number like `26` parsed as Number(26.0) should compare as "26".
            let target_owned;
            let target: &str = match filter_val {
                FilterValue::Text(s) => s.as_str(),
                FilterValue::Number(n) => {
                    // Format as integer when the value is a whole number (e.g. 26.0 → "26").
                    target_owned = if n.fract() == 0.0 {
                        format!("{}", *n as i64)
                    } else {
                        n.to_string()
                    };
                    &target_owned
                }
                // Size values with string ops are not meaningful.
                FilterValue::Size(_) => return false,
            };
            match op {
                FilterOp::StartsWith => val.starts_with(target),
                FilterOp::EndsWith => val.ends_with(target),
                FilterOp::Contains => val.contains(target),
                FilterOp::Regex => {
                    // Compile the regex once per filter evaluation. An invalid
                    // pattern is treated as a non-match rather than a panic.
                    Regex::new(target).is_ok_and(|re| re.is_match(val))
                }
                _ => unreachable!(),
            }
        }
        _ => {
            // Ordering / equality ops.
            let target = match filter_val {
                FilterValue::Text(s) => s.as_str(),
                FilterValue::Number(n) => {
                    // Numeric filter on string field — convert number to string.
                    let s = n.to_string();
                    return match op {
                        FilterOp::Eq => val == s,
                        FilterOp::Ne => val != s,
                        _ => false,
                    };
                }
                FilterValue::Size(bytes) => {
                    let s = bytes.to_string();
                    return match op {
                        FilterOp::Eq => val == s,
                        FilterOp::Ne => val != s,
                        _ => false,
                    };
                }
            };
            match op {
                FilterOp::Eq => val == target,
                FilterOp::Ne => val != target,
                FilterOp::Lt => val < target,
                FilterOp::Le => val <= target,
                FilterOp::Gt => val > target,
                FilterOp::Ge => val >= target,
                _ => unreachable!(),
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SortDirection {
    Best,
    Worst,
}

fn sort_direction(token: &FormatToken) -> SortDirection {
    match token {
        FormatToken::Keyword {
            quality: Quality::Worst,
            ..
        } => SortDirection::Worst,
        _ => SortDirection::Best,
    }
}

/// Rank two formats by quality. Used with `max_by` (best) or `min_by` (worst).
fn rank_formats(token: &FormatToken, a: &Format, b: &Format) -> std::cmp::Ordering {
    match token {
        FormatToken::Keyword {
            stream_type: StreamType::Audio,
            ..
        } => {
            // Audio ranking: abr > asr
            cmp_opt_f64(a.abr, b.abr).then(a.asr.cmp(&b.asr))
        }
        FormatToken::Keyword {
            stream_type: StreamType::Video,
            ..
        } => {
            // Video ranking: height > vbr > fps
            a.height
                .cmp(&b.height)
                .then(cmp_opt_f64(a.vbr, b.vbr))
                .then(cmp_opt_f64(a.fps, b.fps))
        }
        _ => {
            // Combined / general ranking: quality > height > tbr > fps
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
/// When either side is `None`, treat as `Equal` — missing data should not
/// bias the ranking (prevents `Some(x) > None` from `Option::partial_cmp`
/// causing HLS formats with known bitrate to always outrank direct downloads
/// that lack bitrate metadata).
fn cmp_opt_f64(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
        _ => std::cmp::Ordering::Equal,
    }
}
