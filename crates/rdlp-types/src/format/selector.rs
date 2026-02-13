//! Format selection DSL — yt-dlp-compatible expression parser and selector.
//!
//! Grammar:
//!   expression   = format_spec ( "/" format_spec )*        -- fallback chain
//!   format_spec  = selector ( "+" selector )?              -- video+audio merge
//!   selector     = base_name filter*                       -- base with filters
//!   filter       = "[" field op value "]"
//!   base_name    = "best" | "worst" | "b" | "w"
//!                | "bestvideo" | "bv" | "bv*"
//!                | "bestaudio" | "ba" | "ba*"
//!                | "worstvideo" | "wv"
//!                | "worstaudio" | "wa"
//!                | <format_id>
//!   field        = "height" | "width" | "ext" | "vcodec" | "acodec"
//!                | "fps" | "tbr" | "vbr" | "abr" | "asr"
//!                | "filesize" | "protocol" | "format_id"
//!   op           = "<=" | ">=" | "!=" | "<" | ">" | "="
//!   value        = number | string

use super::Format;

/// Parsed format selection expression supporting yt-dlp-compatible syntax.
///
/// # Examples
///
/// ```
/// use rdlp_types::FormatSelector;
///
/// // Basic selectors
/// let sel = FormatSelector::parse("best").unwrap();
/// let sel = FormatSelector::parse("bv+ba").unwrap();
///
/// // Filters
/// let sel = FormatSelector::parse("bv[height<=720]+ba").unwrap();
///
/// // Fallback chains
/// let sel = FormatSelector::parse("bv[ext=mp4]+ba[ext=m4a]/b").unwrap();
/// ```
pub struct FormatSelector {
    expression: String,
    fallbacks: Vec<FormatSpec>,
}

/// A single format specification: either a single selector or a video+audio merge.
#[derive(Debug, Clone, PartialEq)]
enum FormatSpec {
    Single(Selector),
    Merge { video: Selector, audio: Selector },
}

/// A base selector with optional filters.
#[derive(Debug, Clone, PartialEq)]
struct Selector {
    base: BaseSelector,
    filters: Vec<Filter>,
}

/// The base selector type determining which formats are candidates.
#[derive(Debug, Clone, PartialEq)]
enum BaseSelector {
    /// Best combined (video+audio) format
    Best,
    /// Worst combined (video+audio) format
    Worst,
    /// Best video-only format (excludes combined)
    BestVideo,
    /// Best video format (may include combined)
    BestVideoStar,
    /// Worst video-only format
    WorstVideo,
    /// Best audio-only format (excludes combined)
    BestAudio,
    /// Best audio format (may include combined)
    BestAudioStar,
    /// Worst audio-only format
    WorstAudio,
    /// Match a specific format ID
    FormatId(String),
}

/// A filter condition applied to a format field.
#[derive(Debug, Clone, PartialEq)]
struct Filter {
    field: FilterField,
    op: FilterOp,
    value: FilterValue,
}

/// Format fields that can be filtered on.
#[derive(Debug, Clone, PartialEq)]
enum FilterField {
    Height,
    Width,
    Ext,
    Vcodec,
    Acodec,
    Fps,
    Tbr,
    Vbr,
    Abr,
    Asr,
    Filesize,
    Protocol,
    FormatId,
}

/// Comparison operators for filters.
#[derive(Debug, Clone, PartialEq)]
enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A filter value: either a number or a string.
#[derive(Debug, Clone, PartialEq)]
enum FilterValue {
    Number(f64),
    Text(String),
}

impl FormatSelector {
    /// Parse a format selection expression.
    ///
    /// Returns an error if the expression is empty or contains invalid syntax.
    pub fn parse(expression: &str) -> Result<Self, String> {
        let expression = expression.trim();
        if expression.is_empty() {
            return Err("Empty format expression".to_string());
        }

        // Strip trailing '/' (yt-dlp tolerates it)
        let to_parse = expression.strip_suffix('/').unwrap_or(expression);

        let fallbacks = parse_expression
            .parse(to_parse)
            .map_err(|e| format!("Invalid format expression '{expression}': {e}"))?;

        Ok(Self {
            expression: expression.to_string(),
            fallbacks,
        })
    }

    /// Get the original expression string.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Select formats from the available list.
    ///
    /// Tries each fallback in order, returning the first non-empty result.
    /// Returns 1 format for single selectors, 2 for merge (`video+audio`),
    /// or 0 if nothing matches.
    pub fn select<'a>(&self, formats: &'a [Format]) -> Vec<&'a Format> {
        for spec in &self.fallbacks {
            let result = select_spec(spec, formats);
            if !result.is_empty() {
                return result;
            }
        }
        Vec::new()
    }
}

// ---- Parser (winnow combinators) ----

use winnow::combinator::{alt, delimited, opt, repeat, separated};
use winnow::prelude::*;
use winnow::token::take_while;

/// Parse a complete format expression: `format_spec ( "/" format_spec )*`
fn parse_expression(input: &mut &str) -> ModalResult<Vec<FormatSpec>> {
    separated(1.., parse_format_spec, '/').parse_next(input)
}

/// Parse a format spec: `selector` or `selector "+" selector`
fn parse_format_spec(input: &mut &str) -> ModalResult<FormatSpec> {
    let first = parse_selector(input)?;
    if opt('+').parse_next(input)?.is_some() {
        let second = parse_selector(input)?;
        Ok(FormatSpec::Merge {
            video: first,
            audio: second,
        })
    } else {
        Ok(FormatSpec::Single(first))
    }
}

/// Parse a selector: `base_name filter*`
fn parse_selector(input: &mut &str) -> ModalResult<Selector> {
    let base = parse_base_selector(input)?;
    let filters: Vec<Filter> = repeat(0.., parse_filter).parse_next(input)?;
    Ok(Selector { base, filters })
}

/// Parse a base selector keyword or format ID.
fn parse_base_selector(input: &mut &str) -> ModalResult<BaseSelector> {
    alt((
        "bestvideo*".value(BaseSelector::BestVideoStar),
        "bestaudio*".value(BaseSelector::BestAudioStar),
        "bestvideo".value(BaseSelector::BestVideo),
        "bestaudio".value(BaseSelector::BestAudio),
        "worstvideo".value(BaseSelector::WorstVideo),
        "worstaudio".value(BaseSelector::WorstAudio),
        "best".value(BaseSelector::Best),
        "worst".value(BaseSelector::Worst),
        "bv*".value(BaseSelector::BestVideoStar),
        "ba*".value(BaseSelector::BestAudioStar),
        "bv".value(BaseSelector::BestVideo),
        "ba".value(BaseSelector::BestAudio),
        "wv".value(BaseSelector::WorstVideo),
        "wa".value(BaseSelector::WorstAudio),
        "b".value(BaseSelector::Best),
        "w".value(BaseSelector::Worst),
        parse_format_id,
    ))
    .parse_next(input)
}

/// Parse a literal format ID (anything except whitespace and special chars).
fn parse_format_id(input: &mut &str) -> ModalResult<BaseSelector> {
    take_while(1.., |c: char| {
        !c.is_whitespace() && !matches!(c, '+' | '/' | '[' | ']')
    })
    .map(|id: &str| BaseSelector::FormatId(id.to_string()))
    .parse_next(input)
}

/// Parse a single `[field op value]` filter.
fn parse_filter(input: &mut &str) -> ModalResult<Filter> {
    delimited('[', parse_filter_inner, ']').parse_next(input)
}

/// Parse the inside of a filter: `field op value`.
fn parse_filter_inner(input: &mut &str) -> ModalResult<Filter> {
    let field = parse_filter_field(input)?;
    let op = parse_filter_op(input)?;
    let value = parse_filter_value(input)?;
    Ok(Filter { field, op, value })
}

/// Parse a filter field name.
fn parse_filter_field(input: &mut &str) -> ModalResult<FilterField> {
    alt((
        "height".value(FilterField::Height),
        "width".value(FilterField::Width),
        "filesize".value(FilterField::Filesize),
        "format_id".value(FilterField::FormatId),
        "protocol".value(FilterField::Protocol),
        "vcodec".value(FilterField::Vcodec),
        "acodec".value(FilterField::Acodec),
        "ext".value(FilterField::Ext),
        "fps".value(FilterField::Fps),
        "tbr".value(FilterField::Tbr),
        "vbr".value(FilterField::Vbr),
        "abr".value(FilterField::Abr),
        "asr".value(FilterField::Asr),
    ))
    .parse_next(input)
}

/// Parse a comparison operator (two-char operators first to avoid prefix ambiguity).
fn parse_filter_op(input: &mut &str) -> ModalResult<FilterOp> {
    alt((
        "<=".value(FilterOp::Le),
        ">=".value(FilterOp::Ge),
        "!=".value(FilterOp::Ne),
        "<".value(FilterOp::Lt),
        ">".value(FilterOp::Gt),
        "=".value(FilterOp::Eq),
    ))
    .parse_next(input)
}

/// Parse a filter value — try as number first, fall back to text.
fn parse_filter_value(input: &mut &str) -> ModalResult<FilterValue> {
    take_while(1.., |c: char| c != ']')
        .map(|raw: &str| {
            let raw = raw.trim();
            if let Ok(n) = raw.parse::<f64>() {
                FilterValue::Number(n)
            } else {
                FilterValue::Text(raw.to_string())
            }
        })
        .parse_next(input)
}

// ---- Selection logic ----

/// Evaluate a `FormatSpec` against the format list.
fn select_spec<'a>(spec: &FormatSpec, formats: &'a [Format]) -> Vec<&'a Format> {
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
/// OR if codecs are unknown (both `None`) — assumed to be a combined stream.
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
/// Returns `false` if the value is `None` (conservative — missing data doesn't match).
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
            // Numeric filter on string field — convert number to string for comparison
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DownloadProtocol;

    // ---- Test helpers ----

    fn make_combined(id: &str, ext: &str, height: u32, quality: i32) -> Format {
        let mut f = Format::new(id, format!("url_{id}"), ext, DownloadProtocol::Https);
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("aac".to_string());
        f.height = Some(height);
        f.width = Some(height * 16 / 9);
        f.quality = Some(quality);
        f.tbr = Some(height as f64 * 2.0);
        f
    }

    fn make_video_only(id: &str, ext: &str, height: u32) -> Format {
        let mut f = Format::new(id, format!("url_{id}"), ext, DownloadProtocol::Https);
        f.vcodec = Some("h264".to_string());
        f.acodec = Some("none".to_string());
        f.height = Some(height);
        f.width = Some(height * 16 / 9);
        f.vbr = Some(height as f64 * 1.5);
        f
    }

    fn make_audio_only(id: &str, ext: &str, abr: f64) -> Format {
        let mut f = Format::new(id, format!("url_{id}"), ext, DownloadProtocol::Https);
        f.vcodec = Some("none".to_string());
        f.acodec = Some("aac".to_string());
        f.abr = Some(abr);
        f
    }

    fn test_formats() -> Vec<Format> {
        vec![
            make_combined("c360", "mp4", 360, 1),
            make_combined("c720", "mp4", 720, 2),
            make_combined("c1080", "mp4", 1080, 3),
            make_video_only("v720", "mp4", 720),
            make_video_only("v1080", "webm", 1080),
            make_video_only("v1440", "mp4", 1440),
            make_audio_only("a128", "m4a", 128.0),
            make_audio_only("a256", "m4a", 256.0),
            make_audio_only("a64", "webm", 64.0),
        ]
    }

    // ---- Parser tests ----

    #[test]
    fn test_parse_basic_selectors() {
        assert!(FormatSelector::parse("best").is_ok());
        assert!(FormatSelector::parse("worst").is_ok());
        assert!(FormatSelector::parse("b").is_ok());
        assert!(FormatSelector::parse("w").is_ok());
        assert!(FormatSelector::parse("bestvideo").is_ok());
        assert!(FormatSelector::parse("bv").is_ok());
        assert!(FormatSelector::parse("bv*").is_ok());
        assert!(FormatSelector::parse("bestaudio").is_ok());
        assert!(FormatSelector::parse("ba").is_ok());
        assert!(FormatSelector::parse("ba*").is_ok());
        assert!(FormatSelector::parse("worstvideo").is_ok());
        assert!(FormatSelector::parse("worstaudio").is_ok());
    }

    #[test]
    fn test_parse_format_id() {
        let sel = FormatSelector::parse("720p").unwrap();
        assert_eq!(sel.expression(), "720p");
    }

    #[test]
    fn test_parse_merge() {
        assert!(FormatSelector::parse("bv+ba").is_ok());
        assert!(FormatSelector::parse("bestvideo+bestaudio").is_ok());
        assert!(FormatSelector::parse("bv*+ba").is_ok());
    }

    #[test]
    fn test_parse_filters() {
        assert!(FormatSelector::parse("bv[height<=720]").is_ok());
        assert!(FormatSelector::parse("bv[height<=720]+ba[abr>=128]").is_ok());
        assert!(FormatSelector::parse("best[ext=mp4]").is_ok());
        assert!(FormatSelector::parse("bv[height<=1080][ext=mp4]").is_ok());
    }

    #[test]
    fn test_parse_fallback() {
        assert!(FormatSelector::parse("bv+ba/b").is_ok());
        assert!(FormatSelector::parse("bv[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/best").is_ok());
    }

    #[test]
    fn test_parse_errors() {
        assert!(FormatSelector::parse("").is_err());
        assert!(FormatSelector::parse("bv[height<=").is_err());
        assert!(FormatSelector::parse("bv[unknownfield=1]").is_err());
        assert!(FormatSelector::parse("[height<=720]").is_err()); // missing base
        assert!(FormatSelector::parse("bv[height]").is_err()); // no operator
    }

    #[test]
    fn test_parse_empty_fallback_rejected() {
        assert!(FormatSelector::parse("/best").is_err());
        assert!(FormatSelector::parse("best/").is_ok()); // trailing empty is trimmed out by split
    }

    #[test]
    fn test_parse_all_operators() {
        assert!(FormatSelector::parse("bv[height=720]").is_ok());
        assert!(FormatSelector::parse("bv[height!=720]").is_ok());
        assert!(FormatSelector::parse("bv[height<720]").is_ok());
        assert!(FormatSelector::parse("bv[height<=720]").is_ok());
        assert!(FormatSelector::parse("bv[height>720]").is_ok());
        assert!(FormatSelector::parse("bv[height>=720]").is_ok());
    }

    #[test]
    fn test_parse_all_fields() {
        for field in &[
            "height",
            "width",
            "ext",
            "vcodec",
            "acodec",
            "fps",
            "tbr",
            "vbr",
            "abr",
            "asr",
            "filesize",
            "protocol",
            "format_id",
        ] {
            let expr = format!("best[{field}=1]");
            assert!(
                FormatSelector::parse(&expr).is_ok(),
                "Failed to parse field: {field}"
            );
        }
    }

    // ---- Selection tests ----

    #[test]
    fn test_select_best() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080"); // highest quality combined
    }

    #[test]
    fn test_select_worst() {
        let formats = test_formats();
        let sel = FormatSelector::parse("worst").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c360"); // lowest quality combined
    }

    #[test]
    fn test_select_bestvideo() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bestvideo").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v1440"); // highest video-only
    }

    #[test]
    fn test_select_bestaudio() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bestaudio").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a256"); // highest audio-only
    }

    #[test]
    fn test_select_worstvideo() {
        let formats = test_formats();
        let sel = FormatSelector::parse("worstvideo").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v720"); // lowest video-only
    }

    #[test]
    fn test_select_worstaudio() {
        let formats = test_formats();
        let sel = FormatSelector::parse("worstaudio").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a64"); // lowest audio-only
    }

    #[test]
    fn test_select_bv_star_includes_combined() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv*").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        // bv* considers all formats with video, including combined — v1440 has highest height
        assert_eq!(result[0].format_id, "v1440");
    }

    #[test]
    fn test_select_ba_star_includes_combined() {
        let formats = test_formats();
        let sel = FormatSelector::parse("ba*").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        // ba* considers all formats with audio — a256 has highest abr
        assert_eq!(result[0].format_id, "a256");
    }

    #[test]
    fn test_select_merge() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv+ba").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].format_id, "v1440"); // best video-only
        assert_eq!(result[1].format_id, "a256"); // best audio-only
    }

    #[test]
    fn test_select_filter_height_le() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best[height<=720]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720"); // best combined <=720
    }

    #[test]
    fn test_select_filter_height_lt() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best[height<720]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c360"); // only combined <720
    }

    #[test]
    fn test_select_filter_ext() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[ext=webm]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v1080"); // only webm video
    }

    #[test]
    fn test_select_filter_ext_ne() {
        let formats = test_formats();
        let sel = FormatSelector::parse("ba[ext!=m4a]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a64"); // only webm audio
    }

    #[test]
    fn test_select_filter_abr_ge() {
        let formats = test_formats();
        let sel = FormatSelector::parse("ba[abr>=128]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a256"); // best audio >=128
    }

    #[test]
    fn test_select_multiple_filters() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height<=1080][ext=mp4]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v720"); // mp4 video-only <=1080
    }

    #[test]
    fn test_select_merge_with_filters() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height<=720]+ba[abr>=128]").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].format_id, "v720"); // best video <=720
        assert_eq!(result[1].format_id, "a256"); // best audio >=128
    }

    #[test]
    fn test_select_fallback() {
        let formats = test_formats();
        // No video-only format at exactly 360p — falls back to best
        let sel = FormatSelector::parse("bv[height=360]/best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080"); // fallback to best
    }

    #[test]
    fn test_select_fallback_first_matches() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height<=720]/best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "v720"); // first fallback matches
    }

    #[test]
    fn test_select_format_id() {
        let formats = test_formats();
        let sel = FormatSelector::parse("c720").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720");
    }

    #[test]
    fn test_select_drm_excluded() {
        let mut formats = test_formats();
        // Make the best combined format DRM-protected
        formats[2].has_drm = Some(true); // c1080
        let sel = FormatSelector::parse("best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720"); // c1080 excluded, next best
    }

    #[test]
    fn test_select_empty_formats() {
        let formats: Vec<Format> = vec![];
        let sel = FormatSelector::parse("best").unwrap();
        let result = sel.select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_no_match() {
        let formats = test_formats();
        let sel = FormatSelector::parse("bv[height>=4320]").unwrap();
        let result = sel.select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_missing_field_conservative() {
        // Format without fps set — filter on fps should not match
        let mut f = make_combined("test", "mp4", 1080, 5);
        f.fps = None;
        let formats = vec![f];
        let sel = FormatSelector::parse("best[fps>=30]").unwrap();
        let result = sel.select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_shorthand_aliases() {
        let formats = test_formats();

        let sel_b = FormatSelector::parse("b").unwrap();
        let sel_best = FormatSelector::parse("best").unwrap();
        assert_eq!(
            sel_b.select(&formats)[0].format_id,
            sel_best.select(&formats)[0].format_id
        );

        let sel_w = FormatSelector::parse("w").unwrap();
        let sel_worst = FormatSelector::parse("worst").unwrap();
        assert_eq!(
            sel_w.select(&formats)[0].format_id,
            sel_worst.select(&formats)[0].format_id
        );
    }

    #[test]
    fn test_select_complex_expression() {
        let formats = test_formats();
        // "best mp4 video + m4a audio, fallback to best combined mp4, fallback to anything"
        let sel =
            FormatSelector::parse("bv[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4]/best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].format_id, "v720"); // best mp4 video <=1080
        assert_eq!(result[1].format_id, "a256"); // best m4a audio
    }
}
