//! SegmentTemplate / SegmentTimeline / SegmentList → `Vec<Fragment>`.

use rdlp_types::Fragment;

/// Per-Representation segment count ceiling.
///
/// Hard cap protecting against adversarial / malformed MPDs (e.g.
/// `period_duration_seconds = 1e9` or `duration = 0` corner cases that
/// would otherwise saturate to `u64::MAX` and OOM `Vec::with_capacity`).
/// 1M segments at e.g. 4s each is ~46 days of content — well beyond any
/// realistic VoD; truncating at this point is preferable to aborting.
pub(crate) const MAX_SEGMENTS_PER_REP: usize = 1_000_000;

/// Substitute DASH SegmentTemplate placeholders.
///
/// Supported tokens (per ISO/IEC 23009-1 §5.3.9.4.4):
/// - `$$` literal `$`
/// - `$RepresentationID$` → `repr_id`
/// - `$Number$` and `$Number%0Nd$` → segment number with optional width
/// - `$Time$` and `$Time%0Nd$` → segment time
/// - `$Bandwidth$` → bandwidth in bits/sec
pub fn substitute_template(
    template: &str,
    repr_id: &str,
    bandwidth: u64,
    number: Option<u64>,
    time: Option<u64>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        // Found a '$' — find the next '$' to close the token.
        let Some(end_rel) = template[i + 1..].find('$') else {
            out.push('$');
            i += 1;
            continue;
        };
        let end = i + 1 + end_rel;
        let token = &template[i + 1..end];
        let replacement = render_token(token, repr_id, bandwidth, number, time);
        out.push_str(&replacement);
        i = end + 1;
    }
    out
}

fn render_token(
    token: &str,
    repr_id: &str,
    bandwidth: u64,
    number: Option<u64>,
    time: Option<u64>,
) -> String {
    if token.is_empty() {
        return "$".to_string();
    }
    if token == "RepresentationID" {
        return repr_id.to_string();
    }
    if token == "Bandwidth" {
        return bandwidth.to_string();
    }
    let (name, width) = parse_token(token);
    let value = match name {
        "Number" => number.unwrap_or(0),
        "Time" => time.unwrap_or(0),
        other => return format!("${other}$"),
    };
    match width {
        Some(w) => format!("{value:0width$}", width = w),
        None => value.to_string(),
    }
}

fn parse_token(token: &str) -> (&str, Option<usize>) {
    if let Some((name, fmt)) = token.split_once('%') {
        // fmt is e.g. "05d" or "0d" or "d".
        // Strip the trailing 'd' then parse the numeric width directly.
        // The leading '0' is a zero-pad flag, not extra width digits.
        let digits = fmt.trim_end_matches('d');
        let width = digits.parse::<usize>().ok().filter(|w| *w > 0);
        (name, width)
    } else {
        (token, None)
    }
}

/// Plan describing a SegmentTemplate-style fragment list.
#[derive(Debug, Clone)]
pub struct SegmentTemplatePlan {
    pub initialization: Option<String>,
    pub media: String,
    pub start_number: u64,
    pub duration: u64,
    pub timescale: u64,
    pub period_duration_seconds: f64,
}

/// Resolve a SegmentTemplate plan to an ordered fragment list.
///
/// The init segment (if any) is the first entry; media segments follow,
/// numbered from `start_number`.
pub fn resolve_segment_template(
    plan: &SegmentTemplatePlan,
    repr_id: &str,
    bandwidth: u64,
) -> Vec<Fragment> {
    if plan.duration == 0 || plan.timescale == 0 {
        log::warn!(
            "DASH SegmentTemplate has zero duration ({}) or timescale ({}); skipping rep {}",
            plan.duration,
            plan.timescale,
            repr_id,
        );
        return Vec::new();
    }
    if plan.period_duration_seconds <= 0.0 || !plan.period_duration_seconds.is_finite() {
        log::warn!(
            "DASH SegmentTemplate has non-positive or non-finite period duration ({}); skipping rep {}",
            plan.period_duration_seconds,
            repr_id,
        );
        return Vec::new();
    }

    let segment_duration_seconds = plan.duration as f64 / plan.timescale as f64;
    let count_f = (plan.period_duration_seconds / segment_duration_seconds).ceil();
    let count: usize = if count_f.is_finite() && count_f >= 0.0 {
        let raw = count_f as u64;
        // Cap protects against adversarial inputs.
        std::cmp::min(raw as usize, MAX_SEGMENTS_PER_REP)
    } else {
        log::warn!(
            "DASH SegmentTemplate produced non-finite segment count for rep {}; skipping",
            repr_id,
        );
        return Vec::new();
    };
    if count >= MAX_SEGMENTS_PER_REP {
        log::warn!(
            "DASH SegmentTemplate for rep {} would emit ≥{} segments; capping",
            repr_id,
            MAX_SEGMENTS_PER_REP,
        );
    }

    let mut fragments = Vec::with_capacity(count + 1);
    if let Some(init_template) = &plan.initialization {
        let init_url = substitute_template(init_template, repr_id, bandwidth, None, None);
        fragments.push(Fragment {
            url: init_url,
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: None,
            filesize: None,
        });
    }
    for i in 0..count as u64 {
        let number = plan.start_number + i;
        let url = substitute_template(&plan.media, repr_id, bandwidth, Some(number), None);
        fragments.push(Fragment {
            url,
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: Some(segment_duration_seconds),
            filesize: None,
        });
    }
    fragments
}

/// One `<S>` entry from a SegmentTimeline.
#[derive(Debug, Clone)]
pub struct TimelineEntry {
    pub t: Option<u64>,
    pub d: u64,
    pub r: i64,
}

/// Plan describing a SegmentTimeline-style fragment list.
#[derive(Debug, Clone)]
pub struct SegmentTimelinePlan {
    pub initialization: Option<String>,
    pub media: String,
    pub timescale: u64,
    pub entries: Vec<TimelineEntry>,
}

/// Resolve a SegmentTimeline plan to fragments. `$Time$` substitution.
pub fn resolve_segment_timeline(
    plan: &SegmentTimelinePlan,
    repr_id: &str,
    bandwidth: u64,
) -> Vec<Fragment> {
    if plan.timescale == 0 {
        log::warn!(
            "DASH SegmentTimeline has timescale=0; skipping rep {}",
            repr_id
        );
        return Vec::new();
    }

    let mut fragments = Vec::new();
    if let Some(init_template) = &plan.initialization {
        let url = substitute_template(init_template, repr_id, bandwidth, None, None);
        fragments.push(Fragment {
            url,
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: None,
            filesize: None,
        });
    }
    let mut current_time: u64 = 0;
    for entry in &plan.entries {
        if let Some(t) = entry.t {
            current_time = t;
        }
        // Negative-r ("repeat to period end") is deferred — see plan non-goals.
        // TODO(dash): negative-r repeat to period end
        let repeat = if entry.r < 0 { 0 } else { entry.r as u64 };
        for _ in 0..=repeat {
            // Cap protects against adversarial / malformed timelines emitting >MAX segments.
            if fragments.len() >= MAX_SEGMENTS_PER_REP {
                log::warn!(
                    "DASH SegmentTimeline for rep {} exceeds {} segments; capping",
                    repr_id,
                    MAX_SEGMENTS_PER_REP,
                );
                return fragments;
            }
            let url =
                substitute_template(&plan.media, repr_id, bandwidth, None, Some(current_time));
            let duration_seconds = entry.d as f64 / plan.timescale as f64;
            fragments.push(Fragment {
                url,
                byte_range: None,
                init_url: None,
                init_byte_range: None,
                duration: Some(duration_seconds),
                filesize: None,
            });
            // Saturating-add protects against overflow on adversarial input.
            current_time = current_time.saturating_add(entry.d);
        }
    }
    fragments
}

#[cfg(test)]
mod timeline_tests {
    use super::*;

    #[test]
    fn t_d_r_expansion() {
        let plan = SegmentTimelinePlan {
            initialization: None,
            media: "$Time$.m4s".into(),
            timescale: 1_000,
            entries: vec![
                TimelineEntry {
                    t: Some(0),
                    d: 4_000,
                    r: 2,
                },
                TimelineEntry {
                    t: None,
                    d: 2_000,
                    r: 0,
                },
            ],
        };
        let frags = resolve_segment_timeline(&plan, "v", 1);
        // r=2 means 1 + 2 = 3 segments at t=0,4000,8000
        // followed by one at t=12000
        let urls: Vec<&str> = frags.iter().map(|f| f.url.as_str()).collect();
        assert_eq!(urls, vec!["0.m4s", "4000.m4s", "8000.m4s", "12000.m4s"]);
    }

    #[test]
    fn t_default_picks_up_from_prev_end() {
        let plan = SegmentTimelinePlan {
            initialization: None,
            media: "$Time$.m4s".into(),
            timescale: 1_000,
            entries: vec![
                TimelineEntry {
                    t: Some(100),
                    d: 50,
                    r: 0,
                },
                TimelineEntry {
                    t: None,
                    d: 30,
                    r: 1,
                },
            ],
        };
        let frags = resolve_segment_timeline(&plan, "v", 1);
        let urls: Vec<&str> = frags.iter().map(|f| f.url.as_str()).collect();
        // entry 1: t=100, ends at 150
        // entry 2: t=150 (default), 30, r=1 → 150, 180
        assert_eq!(urls, vec!["100.m4s", "150.m4s", "180.m4s"]);
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[test]
    fn ten_segment_period_with_init() {
        let plan = SegmentTemplatePlan {
            initialization: Some("init/$RepresentationID$.m4s".into()),
            media: "seg/$RepresentationID$/$Number%03d$.m4s".into(),
            start_number: 1,
            duration: 4_000,
            timescale: 1_000,
            period_duration_seconds: 40.0,
        };
        let frags = resolve_segment_template(&plan, "v720p", 2_500_000);
        assert_eq!(frags.len(), 11, "1 init + 10 media segments");
        assert_eq!(frags[0].url, "init/v720p.m4s");
        assert_eq!(frags[1].url, "seg/v720p/001.m4s");
        assert_eq!(frags[10].url, "seg/v720p/010.m4s");
    }

    #[test]
    fn ten_segment_period_without_init() {
        let plan = SegmentTemplatePlan {
            initialization: None,
            media: "seg-$Number$.m4s".into(),
            start_number: 0,
            duration: 1,
            timescale: 1,
            period_duration_seconds: 5.0,
        };
        let frags = resolve_segment_template(&plan, "v", 1_000);
        assert_eq!(frags.len(), 5, "no init prepended when missing");
        assert_eq!(frags[0].url, "seg-0.m4s");
    }

    #[test]
    fn zero_duration_returns_empty() {
        let plan = SegmentTemplatePlan {
            initialization: None,
            media: "$Number$.m4s".into(),
            start_number: 1,
            duration: 0,
            timescale: 1_000,
            period_duration_seconds: 40.0,
        };
        let frags = resolve_segment_template(&plan, "v", 1);
        assert!(
            frags.is_empty(),
            "duration=0 must return empty list, not OOM"
        );
    }

    #[test]
    fn zero_timescale_returns_empty() {
        let plan = SegmentTemplatePlan {
            initialization: None,
            media: "$Number$.m4s".into(),
            start_number: 1,
            duration: 4_000,
            timescale: 0,
            period_duration_seconds: 40.0,
        };
        let frags = resolve_segment_template(&plan, "v", 1);
        assert!(
            frags.is_empty(),
            "timescale=0 must return empty list, not silent NaN"
        );
    }

    #[test]
    fn count_capped_at_million() {
        let plan = SegmentTemplatePlan {
            initialization: None,
            media: "$Number$.m4s".into(),
            start_number: 1,
            duration: 1,
            timescale: 1,
            period_duration_seconds: 1e9, // 1 billion seconds at 1s each = 1B segments naive
        };
        let frags = resolve_segment_template(&plan, "v", 1);
        assert_eq!(
            frags.len(),
            super::MAX_SEGMENTS_PER_REP,
            "must cap, not OOM"
        );
    }

    #[test]
    fn negative_period_returns_empty() {
        let plan = SegmentTemplatePlan {
            initialization: None,
            media: "$Number$.m4s".into(),
            start_number: 1,
            duration: 4_000,
            timescale: 1_000,
            period_duration_seconds: -5.0,
        };
        let frags = resolve_segment_template(&plan, "v", 1);
        assert!(frags.is_empty());
    }
}

/// One literal segment URL from a SegmentList.
#[derive(Debug, Clone)]
pub struct SegmentListEntry {
    pub media: String,
    pub duration_seconds: Option<f64>,
}

/// Plan describing a SegmentList-style fragment list.
#[derive(Debug, Clone)]
pub struct SegmentListPlan {
    pub initialization: Option<String>,
    pub entries: Vec<SegmentListEntry>,
}

/// Resolve a SegmentList plan. URLs are literal — no `$…$` substitution.
pub fn resolve_segment_list(plan: &SegmentListPlan) -> Vec<Fragment> {
    // Cap protects against adversarial / malformed lists.
    let total = plan.entries.len() + if plan.initialization.is_some() { 1 } else { 0 };
    if total > MAX_SEGMENTS_PER_REP {
        log::warn!(
            "DASH SegmentList has {} entries; capping at {}",
            total,
            MAX_SEGMENTS_PER_REP,
        );
    }
    let cap = std::cmp::min(total, MAX_SEGMENTS_PER_REP);
    let mut fragments = Vec::with_capacity(cap);
    if let Some(init) = plan
        .initialization
        .as_ref()
        .filter(|_| fragments.len() < MAX_SEGMENTS_PER_REP)
    {
        fragments.push(Fragment {
            url: init.clone(),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: None,
            filesize: None,
        });
    }
    for entry in &plan.entries {
        if fragments.len() >= MAX_SEGMENTS_PER_REP {
            break;
        }
        fragments.push(Fragment {
            url: entry.media.clone(),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: entry.duration_seconds,
            filesize: None,
        });
    }
    fragments
}

#[cfg(test)]
mod list_tests {
    use super::*;

    #[test]
    fn enumerates_literals_with_init() {
        let plan = SegmentListPlan {
            initialization: Some("init.m4s".into()),
            entries: vec![
                SegmentListEntry {
                    media: "seg1.m4s".into(),
                    duration_seconds: Some(4.0),
                },
                SegmentListEntry {
                    media: "seg2.m4s".into(),
                    duration_seconds: Some(4.0),
                },
            ],
        };
        let frags = resolve_segment_list(&plan);
        let urls: Vec<&str> = frags.iter().map(|f| f.url.as_str()).collect();
        assert_eq!(urls, vec!["init.m4s", "seg1.m4s", "seg2.m4s"]);
        assert_eq!(frags[0].duration, None);
        assert_eq!(frags[1].duration, Some(4.0));
    }

    #[test]
    fn enumerates_literals_without_init() {
        let plan = SegmentListPlan {
            initialization: None,
            entries: vec![SegmentListEntry {
                media: "only.m4s".into(),
                duration_seconds: None,
            }],
        };
        let frags = resolve_segment_list(&plan);
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].url, "only.m4s");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_number_with_width() {
        let out = substitute_template(
            "video/$RepresentationID$/seg-$Number%05d$.m4s",
            "v720p",
            2_500_000,
            Some(42),
            None,
        );
        assert_eq!(out, "video/v720p/seg-00042.m4s");
    }

    #[test]
    fn substitutes_bandwidth_and_time() {
        let out = substitute_template(
            "$RepresentationID$/$Bandwidth$/$Time$.m4s",
            "audio_aac",
            128_000,
            None,
            Some(1_234_567),
        );
        assert_eq!(out, "audio_aac/128000/1234567.m4s");
    }

    #[test]
    fn literal_dollar_escapes() {
        let out = substitute_template("a$$b", "x", 1, None, None);
        assert_eq!(out, "a$b");
    }

    #[test]
    fn substitutes_number_without_width() {
        let out = substitute_template("seg-$Number$.m4s", "v", 1, Some(7), None);
        assert_eq!(out, "seg-7.m4s");
    }
}
