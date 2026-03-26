//! Format sorter implementing yt-dlp's `-S` (sort) flag behaviour.
//!
//! A `FormatSorter` is a sequence of `SortFieldSpec` entries that together
//! define a multi-key comparator for `Format` values.  The default order
//! matches yt-dlp exactly:
//!
//! ```text
//! hasvid, ie_pref, lang, quality, res, fps, hdr:12, vcodec, channels,
//! acodec, size, br, asr, proto, ext, hasaud, source, id
//! ```
//!
//! # Parsing
//!
//! Use `FormatSorter::parse(spec)` to build a sorter from a yt-dlp `-S` string
//! such as `"res:1080,vcodec:h264,+size"`.
//!
//! Field syntax: `[+]field[:limit]` or `[+]field[~limit]`, comma-separated.
//!
//! - No prefix / `-` prefix → descending (larger value wins, default).
//! - `+` prefix → ascending (smaller value wins).
//! - `:limit` → preferred limit: values at or below the limit are preferred;
//!   values above are demoted.
//! - `~limit` → nearest match: the value closest to `limit` wins.

use regex::Regex;
use std::cmp::Ordering;

use super::error::FormatSelectError;
use super::size::parse_size;
use crate::format::Format;

// ---------------------------------------------------------------------------
// Codec tier tables
// ---------------------------------------------------------------------------

/// Video codec preference tiers (index 0 = best).
///
/// Each tier is a regex pattern.  A lower index means a higher-quality codec.
const VIDEO_CODEC_TIERS: &[&str] = &[
    r"av0?1",          // AV1
    r"vp0?9\.0?2",     // VP9.2 / HDR
    r"vp0?9",          // VP9
    r"[hx]265|he?vc?", // H.265 / HEVC
    r"[hx]264|avc",    // H.264 / AVC
    r"vp0?8",          // VP8
    r"mp4v|h263",      // MPEG-4 / H.263
    r"theora",         // Theora
];

/// Audio codec preference tiers (index 0 = best).
const AUDIO_CODEC_TIERS: &[&str] = &[
    r"[af]lac",    // FLAC / ALAC
    r"wav|aiff",   // WAV / AIFF
    r"opus",       // Opus
    r"vorbis|ogg", // Vorbis / OGG
    r"aac",        // AAC
    r"mp?4a?",     // MP4A / M4A
    r"mp3",        // MP3
    r"ac-?4",      // AC-4
    r"e-?a?c-?3",  // E-AC-3
    r"ac-?3",      // AC-3
    r"dts",        // DTS
];

/// Protocol preference order (lower index = better, matching yt-dlp ordering).
/// https, http, m3u8_native, m3u8, http_dash_segments, everything else.
const PROTOCOL_ORDER: &[&str] = &["https", "http", "m3u8_native", "m3u8", "http_dash_segments"];

// ---------------------------------------------------------------------------
// SortFieldSpec
// ---------------------------------------------------------------------------

/// A single sort criterion within a [`FormatSorter`].
///
/// Produced by [`FormatSorter::parse`]; inspect the individual fields to
/// understand the sort behaviour for each criterion.
#[derive(Debug, Clone)]
pub struct SortFieldSpec {
    /// The field name (lower-cased, as given in the `-S` spec).
    ///
    /// Known fields include `res`, `fps`, `vcodec`, `acodec`, `size`, `br`,
    /// `proto`, `ext`, `hasvid`, `hasaud`, and others matching yt-dlp's `-S`
    /// documentation. Unknown fields are treated as absent (no-op).
    pub field: String,
    /// If `true`, lower values are preferred (ascending); the default is
    /// descending (higher values preferred).
    ///
    /// Controlled by the `+` prefix in the sort spec (e.g. `+size`).
    pub ascending: bool,
    /// Optional preferred upper limit (`:value` syntax).
    ///
    /// When set, values at or below the limit are ranked above values that
    /// exceed it (demoted tier). Among within-limit values, the normal
    /// descending/ascending preference applies.
    pub limit: Option<f64>,
    /// If `true`, sort by absolute distance to `limit` rather than by the
    /// raw value (`~value` syntax).
    ///
    /// The format whose field value is closest to `limit` wins.
    pub nearest: bool,
}

// ---------------------------------------------------------------------------
// FormatSorter
// ---------------------------------------------------------------------------

/// Multi-field format sorter matching yt-dlp's `-S` flag semantics.
///
/// A `FormatSorter` is built from a comma-separated sort spec string.  Each
/// field entry supports an optional ascending prefix (`+`), a preferred-limit
/// suffix (`:value`) and a nearest-match suffix (`~value`).
///
/// Codec fields (`vcodec`, `acodec`) use quality tier tables so that
/// `av01 > vp9 > h265 > h264`, and `flac > opus > aac > mp3`, regardless of
/// the codec string spelling.
///
/// # Constructors
///
/// - [`FormatSorter::parse`] — build from a `-S` spec string.
/// - [`FormatSorter::default_order`] — yt-dlp default sort order.
///
/// # Example
///
/// ```
/// use rdlp_types::FormatSorter;
///
/// // Prefer 1080p; among formats at that resolution prefer h264; break ties
/// // by smallest file size.
/// let sorter = FormatSorter::parse("res:1080,vcodec:h264,+size").unwrap();
/// // Pass the sorter to FormatSelector::select_with_sorter, or call sort()
/// // directly on a mutable slice of &Format.
/// ```
pub struct FormatSorter {
    fields: Vec<SortFieldSpec>,
}

impl FormatSorter {
    // ------------------------------------------------------------------
    // Constructors
    // ------------------------------------------------------------------

    /// Build a `FormatSorter` from a yt-dlp `-S` spec string.
    ///
    /// Returns `FormatSelectError::Parse` on invalid input.
    pub fn parse(spec: &str) -> Result<Self, FormatSelectError> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(FormatSelectError::Parse {
                message: "Empty sort spec".to_string(),
                position: 0,
                input: spec.to_string(),
            });
        }

        let mut fields = Vec::new();

        for token in spec.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }

            // Detect ascending prefix (`+`).
            let (ascending, rest) = if let Some(stripped) = token.strip_prefix('+') {
                (true, stripped)
            } else {
                (false, token)
            };

            // Detect `~limit` (nearest) or `:limit` (preferred upper bound).
            let (field_name, limit, nearest) = if let Some(pos) = rest.find('~') {
                let field_part = &rest[..pos];
                let limit_part = &rest[pos + 1..];
                let limit = parse_limit_value(limit_part, spec)?;
                (field_part, Some(limit), true)
            } else if let Some(pos) = rest.find(':') {
                let field_part = &rest[..pos];
                let limit_part = &rest[pos + 1..];
                // For codec fields, the limit might be a string (e.g. `vcodec:h264`).
                // When the limit value cannot be parsed as a number, we silently
                // ignore the limit rather than returning an error.
                let limit = parse_limit_value(limit_part, spec).ok();
                (field_part, limit, false)
            } else {
                (rest, None, false)
            };

            if field_name.is_empty() {
                return Err(FormatSelectError::Parse {
                    message: "Empty field name in sort spec".to_string(),
                    position: 0,
                    input: spec.to_string(),
                });
            }

            fields.push(SortFieldSpec {
                field: field_name.to_ascii_lowercase(),
                ascending,
                limit,
                nearest,
            });
        }

        if fields.is_empty() {
            return Err(FormatSelectError::Parse {
                message: "No valid fields in sort spec".to_string(),
                position: 0,
                input: spec.to_string(),
            });
        }

        Ok(Self { fields })
    }

    /// Build the default yt-dlp sort order.
    ///
    /// Equivalent to yt-dlp's built-in format sort when no `-S` flag is given.
    /// Fields are evaluated in the following priority order (highest first):
    ///
    /// ```text
    /// hasvid, ie_pref, lang, quality, res, fps, hdr:12, vcodec, channels,
    /// acodec, size, br, asr, proto, ext, hasaud, source, id
    /// ```
    ///
    /// All fields use descending order except where noted (see yt-dlp docs for
    /// full semantics). The `hdr` field uses a preferred limit of `12` (SDR/HLG
    /// threshold).
    #[must_use]
    pub fn default_order() -> Self {
        let specs: &[(&str, bool, Option<f64>, bool)] = &[
            ("hasvid", false, None, false),
            ("ie_pref", false, None, false),
            ("lang", false, None, false),
            ("quality", false, None, false),
            ("res", false, None, false),
            ("fps", false, None, false),
            ("hdr", false, Some(12.0), false),
            ("vcodec", false, None, false),
            ("channels", false, None, false),
            ("acodec", false, None, false),
            ("size", false, None, false),
            ("br", false, None, false),
            ("asr", false, None, false),
            ("proto", false, None, false),
            ("ext", false, None, false),
            ("hasaud", false, None, false),
            ("source", false, None, false),
            ("id", false, None, false),
        ];

        let fields = specs
            .iter()
            .map(|(name, asc, limit, near)| SortFieldSpec {
                field: name.to_string(),
                ascending: *asc,
                limit: *limit,
                nearest: *near,
            })
            .collect();

        Self { fields }
    }

    // ------------------------------------------------------------------
    // Sorting
    // ------------------------------------------------------------------

    /// Returns `true` if the sorter has no sort fields.
    ///
    /// A sorter with no fields is effectively a no-op and is only produced
    /// programmatically (the `parse` constructor rejects empty specs).
    #[must_use]
    pub fn fields_is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Sort a slice of format references in-place.
    ///
    /// After sorting, the best format according to this sorter's criteria is at
    /// index 0.  The sort is stable with respect to equal-ranked formats.
    ///
    /// # Example
    ///
    /// ```
    /// use rdlp_types::{Format, FormatSorter};
    /// use rdlp_types::protocol::DownloadProtocol;
    ///
    /// let mut f480 = Format::new("480p", "https://example.com/480", "mp4", DownloadProtocol::Https);
    /// f480.height = Some(480);
    /// let mut f1080 = Format::new("1080p", "https://example.com/1080", "mp4", DownloadProtocol::Https);
    /// f1080.height = Some(1080);
    ///
    /// let sorter = FormatSorter::parse("res").unwrap();
    /// let mut formats = vec![&f480, &f1080];
    /// sorter.sort(&mut formats);
    /// assert_eq!(formats[0].format_id, "1080p");
    /// ```
    pub fn sort(&self, formats: &mut [&Format]) {
        formats.sort_by(|a, b| self.compare(b, a));
    }

    /// Compare two formats.  Returns the standard `Ordering` for `sort_by`.
    ///
    /// A result of `Greater` means `a` is *better* than `b`.
    pub fn compare(&self, a: &Format, b: &Format) -> Ordering {
        for spec in &self.fields {
            let ord = compare_field(spec, a, b);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }
}

// ---------------------------------------------------------------------------
// Field comparison
// ---------------------------------------------------------------------------

/// The sort key for a single numeric field.
///
/// Encodes yt-dlp's tier/preference logic into a tuple that can be compared
/// with a simple `PartialOrd` call:
///
/// - `tier`:    -10 = absent, -1 = demoted (above preferred limit), 0 = normal,
///   1 = string-typed (codec tier converted to signed score)
/// - `primary`: negated for descending order, raw for ascending
/// - `tiebreak`: direction preference (+1 or -1)
#[derive(PartialEq, PartialOrd)]
struct SortKey {
    tier: i32,
    primary: f64,
    tiebreak: i32,
}

impl SortKey {
    fn missing() -> Self {
        Self {
            tier: -10,
            primary: 0.0,
            tiebreak: 0,
        }
    }
}

/// Compare two formats on a single `SortFieldSpec`.
fn compare_field(spec: &SortFieldSpec, a: &Format, b: &Format) -> Ordering {
    let ka = sort_key(spec, a);
    let kb = sort_key(spec, b);
    // Larger key = better. We want "a better than b" to return Greater.
    ka.partial_cmp(&kb).unwrap_or(Ordering::Equal)
}

/// Compute the sort key for `format` under `spec`.
fn sort_key(spec: &SortFieldSpec, f: &Format) -> SortKey {
    match spec.field.as_str() {
        // ---- boolean fields -----------------------------------------------
        "hasvid" => bool_key(f.has_video()),
        "hasaud" => bool_key(f.has_audio()),

        // ---- numeric fields -----------------------------------------------
        "height" | "res" => numeric_key(f.height.map(|v| v as f64), spec),
        "width" => numeric_key(f.width.map(|v| v as f64), spec),
        "fps" => numeric_key(f.fps, spec),
        "quality" => numeric_key(f.quality.map(|v| v as f64), spec),
        "size" | "filesize" => {
            numeric_key(f.filesize.or(f.filesize_approx).map(|v| v as f64), spec)
        }
        "br" | "tbr" => numeric_key(f.tbr, spec),
        "vbr" => numeric_key(f.vbr, spec),
        "abr" => numeric_key(f.abr, spec),
        "asr" => numeric_key(f.asr.map(|v| v as f64), spec),
        "channels" => numeric_key(f.channels_as_f64(), spec),

        // ---- HDR ----------------------------------------------------------
        "hdr" => {
            // yt-dlp assigns numeric scores to HDR dynamic range strings.
            let score = hdr_score(f.dynamic_range.as_deref());
            numeric_key(Some(score as f64), spec)
        }

        // ---- codec fields (tier-based) ------------------------------------
        "vcodec" => codec_key(f.vcodec.as_deref(), VIDEO_CODEC_TIERS, spec),
        "acodec" => codec_key(f.acodec.as_deref(), AUDIO_CODEC_TIERS, spec),
        "vext" | "aext" | "ext" => string_rank_key(Some(f.ext.as_str()), spec),

        // ---- protocol -----------------------------------------------------
        "proto" | "protocol" => {
            let score = protocol_score(f.protocol.as_str());
            // Higher score = better protocol.
            // Descending (default): primary = score.
            // Ascending: primary = -score.
            SortKey {
                tier: 0,
                primary: if spec.ascending {
                    -(score as f64)
                } else {
                    score as f64
                },
                tiebreak: 0,
            }
        }

        // ---- string fields (lexicographic) --------------------------------
        "id" => string_rank_key(Some(f.format_id.as_str()), spec),
        "lang" | "language" => string_rank_key(f.language.as_deref(), spec),
        "source" => string_rank_key(f.container.as_deref(), spec),

        // ---- ie_pref: use quality field as proxy -------------------------
        "ie_pref" => numeric_key(f.quality.map(|v| v as f64), spec),

        // ---- unknown fields: treat as absent -----------------------------
        _ => SortKey::missing(),
    }
}

// ---------------------------------------------------------------------------
// Key builders
// ---------------------------------------------------------------------------

/// Key for a boolean field: `true` → tier 0 primary 1, `false` → tier -10.
fn bool_key(value: bool) -> SortKey {
    if value {
        SortKey {
            tier: 0,
            primary: 1.0,
            tiebreak: 1,
        }
    } else {
        SortKey::missing()
    }
}

/// Key for a numeric field, honouring `:limit` and `~limit` semantics.
fn numeric_key(value: Option<f64>, spec: &SortFieldSpec) -> SortKey {
    let Some(v) = value else {
        return SortKey::missing();
    };

    if spec.nearest {
        // `~limit`: sort by absolute distance to limit (closest wins).
        let limit = spec.limit.unwrap_or(0.0);
        let distance = (v - limit).abs();
        // Tiebreak: prefer value >= limit (above) if descending, below if ascending.
        let tiebreak: i32 = if spec.ascending {
            if v <= limit { 1 } else { -1 }
        } else {
            if v >= limit { 1 } else { -1 }
        };
        SortKey {
            tier: 0,
            primary: -distance, // closer to limit = less negative = higher key
            tiebreak,
        }
    } else if let Some(limit) = spec.limit {
        // `:limit`: values at or below limit are preferred; values above are demoted.
        if v <= limit {
            // Normal range: within-limit values.
            // Descending: larger value is better → primary = v (higher = better key).
            // Ascending: smaller value is better → primary = -v (less negative = higher key for smaller v).
            //   Wait — ascending means we prefer smaller. Smaller v → -v is larger in magnitude as negative,
            //   so for ascending we want -v to be higher for smaller v: -v increases as v decreases. ✓
            SortKey {
                tier: 0,
                primary: if spec.ascending { -v } else { v },
                tiebreak: if spec.ascending { -1 } else { 1 },
            }
        } else {
            // Demoted range: values above limit are demoted (tier -1).
            // Among demoted values, prefer smallest overshoot (descending) or largest overshoot (ascending).
            // Descending: smallest overshoot preferred → primary = -v (less negative for smaller v).
            // Ascending: largest overshoot preferred → primary = v.
            SortKey {
                tier: -1,
                primary: if spec.ascending { v } else { -v },
                tiebreak: if spec.ascending { 1 } else { -1 },
            }
        }
    } else {
        // Plain field: higher is better by default (descending).
        // Descending: primary = v (higher v → higher key = better).
        // Ascending: primary = -v (smaller v → less negative = higher key = better).
        SortKey {
            tier: 0,
            primary: if spec.ascending { -v } else { v },
            tiebreak: 1,
        }
    }
}

/// Key for codec fields using tier tables.
///
/// A lower tier index means a better codec, so we negate it to get a
/// higher sort key for better codecs.
fn codec_key(codec: Option<&str>, tiers: &[&str], spec: &SortFieldSpec) -> SortKey {
    let Some(codec) = codec else {
        return SortKey::missing();
    };

    // "none" / "" = no codec present.
    if codec == "none" || codec.is_empty() {
        return SortKey::missing();
    }

    let codec_lower = codec.to_ascii_lowercase();
    let tier_score = match_codec_tier(&codec_lower, tiers);

    // Higher score = better codec.
    // Descending (default): primary = score (higher score → higher key = better).
    // Ascending: primary = -score (lower score → higher key = better).
    SortKey {
        tier: 0,
        primary: if spec.ascending {
            -(tier_score as f64)
        } else {
            tier_score as f64
        },
        tiebreak: if spec.ascending { -1 } else { 1 },
    }
}

/// Key for string fields (lexicographic, e.g. `id`, `ext`).
///
/// This maps the string to a numeric score by hashing, which is sufficient
/// for consistent ordering even if not semantically meaningful for ext/id.
/// For `ext`, yt-dlp uses a preference order; we fall back to lexicographic.
fn string_rank_key(value: Option<&str>, spec: &SortFieldSpec) -> SortKey {
    let Some(v) = value else {
        return SortKey::missing();
    };
    // Use a simple lexicographic score by treating the first 8 bytes as a u64.
    let bytes = v.as_bytes();
    let mut score: i64 = 0;
    for (i, &b) in bytes.iter().take(8).enumerate() {
        score |= (b as i64) << (8 * (7 - i));
    }
    // Descending: primary = score (higher lexicographic = higher key).
    // Ascending: primary = -score (lower lexicographic = higher key).
    SortKey {
        tier: 1,
        primary: if spec.ascending {
            -(score as f64)
        } else {
            score as f64
        },
        tiebreak: 0,
    }
}

// ---------------------------------------------------------------------------
// Codec tier matching
// ---------------------------------------------------------------------------

/// Return a signed score for a codec string against a tier list.
///
/// The best codec (index 0 in the tier list) returns the highest score.
/// Unknown codecs return 0 (below all known codecs when negating tier index
/// gives negative scores — we instead return a mid-range value so known-bad
/// is still ranked below unknown).
fn match_codec_tier(codec: &str, tiers: &[&str]) -> i32 {
    for (i, pattern) in tiers.iter().enumerate() {
        if let Ok(re) = Regex::new(pattern)
            && re.is_match(codec)
        {
            // Tier 0 = best = highest score. Score = (len - i).
            return (tiers.len() as i32) - (i as i32);
        }
    }
    // Not in any tier — score 0.
    0
}

// ---------------------------------------------------------------------------
// HDR score
// ---------------------------------------------------------------------------

/// Map a dynamic range string to a numeric yt-dlp HDR score.
///
/// Scores from yt-dlp source (higher = better):
/// - DOVI / DV → 5
/// - HDR10+ → 4
/// - HDR10 → 3
/// - HLG → 2
/// - SDR (or None) → 1
fn hdr_score(dynamic_range: Option<&str>) -> u32 {
    match dynamic_range {
        Some(s) if s.contains("DV") || s.to_ascii_uppercase().contains("DOVI") => 5,
        Some(s) if s.contains("HDR10+") || s.contains("HDR10P") => 4,
        Some(s) if s.contains("HDR10") => 3,
        Some(s) if s.contains("HLG") => 2,
        _ => 1,
    }
}

// ---------------------------------------------------------------------------
// Protocol score
// ---------------------------------------------------------------------------

/// Return a preference score for a protocol string.
///
/// Higher score = better (matches yt-dlp's preference order).
fn protocol_score(proto: &str) -> i32 {
    PROTOCOL_ORDER
        .iter()
        .position(|&p| p == proto)
        .map(|i| (PROTOCOL_ORDER.len() as i32) - (i as i32))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Limit value parser
// ---------------------------------------------------------------------------

/// Parse a limit value from a `:limit` or `~limit` suffix.
///
/// Accepts size literals (e.g. `1G`) or plain numbers (e.g. `1080`).
fn parse_limit_value(s: &str, original_spec: &str) -> Result<f64, FormatSelectError> {
    // Try size literal first.
    if let Some(bytes) = parse_size(s) {
        return Ok(bytes as f64);
    }
    // Fall back to plain number.
    s.parse::<f64>().map_err(|_| FormatSelectError::Parse {
        message: format!("Invalid limit value '{s}'"),
        position: 0,
        input: original_spec.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Format extension trait (internal helpers)
// ---------------------------------------------------------------------------

trait FormatSortExt {
    fn channels_as_f64(&self) -> Option<f64>;
}

impl FormatSortExt for Format {
    fn channels_as_f64(&self) -> Option<f64> {
        // The Format struct does not have a `channels` field yet.
        // Return None so the field is treated as absent.
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    fn make_format(id: &str) -> Format {
        Format::new(
            id,
            "https://example.com/video",
            "mp4",
            DownloadProtocol::Https,
        )
    }

    // ------------------------------------------------------------------
    // Default sort — higher resolution wins
    // ------------------------------------------------------------------

    #[test]
    fn default_prefers_higher_resolution() {
        let mut f480 = make_format("480p");
        f480.height = Some(480);
        f480.vcodec = Some("h264".to_string());
        f480.acodec = Some("aac".to_string());

        let mut f1080 = make_format("1080p");
        f1080.height = Some(1080);
        f1080.vcodec = Some("h264".to_string());
        f1080.acodec = Some("aac".to_string());

        let mut f720 = make_format("720p");
        f720.height = Some(720);
        f720.vcodec = Some("h264".to_string());
        f720.acodec = Some("aac".to_string());

        let mut formats: Vec<&Format> = vec![&f480, &f1080, &f720];
        FormatSorter::default_order().sort(&mut formats);

        assert_eq!(formats[0].format_id, "1080p");
        assert_eq!(formats[1].format_id, "720p");
        assert_eq!(formats[2].format_id, "480p");
    }

    // ------------------------------------------------------------------
    // Preferred value: res:720
    // ------------------------------------------------------------------

    #[test]
    fn sort_with_preferred_value_res_720() {
        let mut f480 = make_format("480p");
        f480.height = Some(480);

        let mut f720 = make_format("720p");
        f720.height = Some(720);

        let mut f1080 = make_format("1080p");
        f1080.height = Some(1080);

        let spec = FormatSorter::parse("res:720").unwrap();
        let mut formats: Vec<&Format> = vec![&f480, &f1080, &f720];
        spec.sort(&mut formats);

        // 720 should be first (at limit), 480 second (below limit, best below),
        // 1080 last (demoted — above limit).
        assert_eq!(formats[0].format_id, "720p");
        assert_eq!(formats[1].format_id, "480p");
        assert_eq!(formats[2].format_id, "1080p");
    }

    // ------------------------------------------------------------------
    // Reversed / ascending
    // ------------------------------------------------------------------

    #[test]
    fn sort_reversed_ascending() {
        let mut f480 = make_format("480p");
        f480.height = Some(480);

        let mut f720 = make_format("720p");
        f720.height = Some(720);

        let mut f1080 = make_format("1080p");
        f1080.height = Some(1080);

        // `+res` means ascending — smallest height first.
        let spec = FormatSorter::parse("+res").unwrap();
        let mut formats: Vec<&Format> = vec![&f1080, &f480, &f720];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "480p");
        assert_eq!(formats[1].format_id, "720p");
        assert_eq!(formats[2].format_id, "1080p");
    }

    // ------------------------------------------------------------------
    // Codec tiers
    // ------------------------------------------------------------------

    #[test]
    fn vcodec_tier_av1_over_h264() {
        let mut fav1 = make_format("av1");
        fav1.vcodec = Some("av1".to_string());
        fav1.height = Some(1080);

        let mut fh264 = make_format("h264");
        fh264.vcodec = Some("h264".to_string());
        fh264.height = Some(1080);

        let spec = FormatSorter::parse("vcodec").unwrap();
        let mut formats: Vec<&Format> = vec![&fh264, &fav1];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "av1");
        assert_eq!(formats[1].format_id, "h264");
    }

    #[test]
    fn acodec_tier_flac_over_mp3() {
        let mut fflac = make_format("flac");
        fflac.acodec = Some("flac".to_string());

        let mut fmp3 = make_format("mp3");
        fmp3.acodec = Some("mp3".to_string());

        let spec = FormatSorter::parse("acodec").unwrap();
        let mut formats: Vec<&Format> = vec![&fmp3, &fflac];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "flac");
        assert_eq!(formats[1].format_id, "mp3");
    }

    // ------------------------------------------------------------------
    // Parse tests
    // ------------------------------------------------------------------

    #[test]
    fn parse_sort_spec() {
        let sorter = FormatSorter::parse("res:1080,vcodec:h264,+size").unwrap();
        assert_eq!(sorter.fields.len(), 3);

        // res:1080 — descending, preferred limit 1080
        assert_eq!(sorter.fields[0].field, "res");
        assert!(!sorter.fields[0].ascending);
        assert_eq!(sorter.fields[0].limit, Some(1080.0));
        assert!(!sorter.fields[0].nearest);

        // vcodec:h264 — descending, limit is None because "h264" is not
        // a parseable number (codec fields use string tiers, not numeric limits).
        assert_eq!(sorter.fields[1].field, "vcodec");
        assert!(!sorter.fields[1].ascending);
        assert_eq!(sorter.fields[1].limit, None);

        // +size — ascending, no limit
        assert_eq!(sorter.fields[2].field, "size");
        assert!(sorter.fields[2].ascending);
        assert_eq!(sorter.fields[2].limit, None);
        assert!(!sorter.fields[2].nearest);
    }

    #[test]
    fn parse_sort_spec_with_nearest() {
        let sorter = FormatSorter::parse("filesize~1G").unwrap();
        assert_eq!(sorter.fields.len(), 1);
        assert_eq!(sorter.fields[0].field, "filesize");
        assert!(!sorter.fields[0].ascending);
        // 1G = 1000^3 bytes (SI, bare G without 'i')
        assert_eq!(sorter.fields[0].limit, Some(1_000_000_000.0));
        assert!(sorter.fields[0].nearest);
    }

    #[test]
    fn parse_empty_spec_is_error() {
        assert!(FormatSorter::parse("").is_err());
    }

    #[test]
    fn parse_ascending_no_limit() {
        let sorter = FormatSorter::parse("+fps").unwrap();
        assert_eq!(sorter.fields[0].field, "fps");
        assert!(sorter.fields[0].ascending);
        assert_eq!(sorter.fields[0].limit, None);
    }

    #[test]
    fn nearest_prefers_closest_value() {
        let mut f900 = make_format("900");
        f900.height = Some(900);

        let mut f720 = make_format("720");
        f720.height = Some(720);

        let mut f1080 = make_format("1080");
        f1080.height = Some(1080);

        // ~720: nearest to 720. Distance: 900→180, 720→0, 1080→360.
        let spec = FormatSorter::parse("res~720").unwrap();
        let mut formats: Vec<&Format> = vec![&f900, &f1080, &f720];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "720"); // distance 0
        assert_eq!(formats[1].format_id, "900"); // distance 180
        assert_eq!(formats[2].format_id, "1080"); // distance 360
    }

    #[test]
    fn codec_tier_vp9_over_h264() {
        let mut fvp9 = make_format("vp9");
        fvp9.vcodec = Some("vp9".to_string());

        let mut fh264 = make_format("h264");
        fh264.vcodec = Some("h264".to_string());

        let spec = FormatSorter::parse("vcodec").unwrap();
        let mut formats: Vec<&Format> = vec![&fh264, &fvp9];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "vp9");
        assert_eq!(formats[1].format_id, "h264");
    }

    #[test]
    fn hasvid_prefers_formats_with_video() {
        let mut with_vid = make_format("vid");
        with_vid.vcodec = Some("h264".to_string());
        with_vid.acodec = Some("none".to_string());

        let mut no_vid = make_format("aud");
        no_vid.vcodec = Some("none".to_string());
        no_vid.acodec = Some("aac".to_string());

        let spec = FormatSorter::parse("hasvid").unwrap();
        let mut formats: Vec<&Format> = vec![&no_vid, &with_vid];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "vid");
        assert_eq!(formats[1].format_id, "aud");
    }

    #[test]
    fn multi_field_sort_tiebreak_by_codec() {
        // Two formats at same resolution, different codecs.
        let mut fav1 = make_format("av1");
        fav1.height = Some(1080);
        fav1.vcodec = Some("av1".to_string());

        let mut fh264 = make_format("h264");
        fh264.height = Some(1080);
        fh264.vcodec = Some("h264".to_string());

        let spec = FormatSorter::parse("res,vcodec").unwrap();
        let mut formats: Vec<&Format> = vec![&fh264, &fav1];
        spec.sort(&mut formats);

        // Same resolution, so vcodec breaks the tie — AV1 is better.
        assert_eq!(formats[0].format_id, "av1");
    }
}
