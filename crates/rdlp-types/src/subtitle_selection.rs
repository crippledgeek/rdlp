//! Subtitle selection logic for choosing which subtitles to download.
//!
//! Provides functions to select subtitles from available options based on
//! user preferences (language, format) and to generate subtitle filenames
//! following the yt-dlp convention.

use std::collections::HashMap;

use crate::info_dict::Subtitle;
use crate::subtitle_format::SubtitleFormat;

/// Select subtitles from available options based on user preferences.
///
/// # Arguments
///
/// * `subtitles` - Available manual subtitles (lang -> list of subtitle entries)
/// * `auto_captions` - Auto-generated captions (lang -> list)
/// * `langs` - Requested language codes (e.g., `["en", "es"]`).
///   Empty = no selection. `"all"` = all languages.
/// * `preferred_format` - Preferred subtitle format (e.g., `SubtitleFormat::Srt`)
/// * `include_auto` - Whether to include automatic captions
///
/// # Returns
///
/// Vec of `(language, Subtitle)` pairs to download.
///
/// # Selection rules
///
/// 1. If `langs` is empty, return empty (no subtitles requested).
/// 2. If `langs` contains `"all"`, include all available languages.
/// 3. For each requested lang, find matching subtitles.
/// 4. If `preferred_format` is set, pick that format first; fallback to first available.
/// 5. If `include_auto` is true, also search `auto_captions` for missing langs.
/// 6. Manual subtitles are always preferred over auto-generated ones.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use rdlp_types::info_dict::Subtitle;
/// use rdlp_types::subtitle_selection::select_subtitles;
///
/// let mut subs = HashMap::new();
/// subs.insert("en".to_string(), vec![Subtitle {
///     url: "https://example.com/en.srt".to_string(),
///     ext: "srt".to_string(),
///     name: Some("English".to_string()),
/// }]);
///
/// let result = select_subtitles(
///     &subs,
///     None,
///     &["en".to_string()],
///     None,
///     false,
/// );
/// assert_eq!(result.len(), 1);
/// assert_eq!(result[0].0, "en");
/// ```
#[must_use]
pub fn select_subtitles(
    subtitles: &HashMap<String, Vec<Subtitle>>,
    auto_captions: Option<&HashMap<String, Vec<Subtitle>>>,
    langs: &[String],
    preferred_format: Option<SubtitleFormat>,
    include_auto: bool,
) -> Vec<(String, Subtitle)> {
    if langs.is_empty() {
        return Vec::new();
    }

    let select_all = langs.iter().any(|l| l.eq_ignore_ascii_case("all"));
    let mut result = Vec::new();

    if select_all {
        // Collect all languages from manual subtitles
        for (lang, subs) in subtitles {
            if let Some(sub) = pick_subtitle(subs, preferred_format) {
                result.push((lang.clone(), sub));
            }
        }
        // Also include auto-captions languages not already covered
        if include_auto && let Some(auto) = auto_captions {
            for (lang, subs) in auto {
                if result.iter().any(|(l, _)| l.eq_ignore_ascii_case(lang)) {
                    continue;
                }
                if let Some(sub) = pick_subtitle(subs, preferred_format) {
                    result.push((lang.clone(), sub));
                }
            }
        }
    } else {
        for requested in langs {
            // Try manual subtitles first
            let manual = find_lang(subtitles, requested);
            if let Some((lang, subs)) = manual {
                if let Some(sub) = pick_subtitle(subs, preferred_format) {
                    result.push((lang.to_owned(), sub));
                    continue;
                }
            }

            // Fallback to auto-captions if enabled
            if include_auto
                && let Some(auto) = auto_captions
                && let Some((lang, subs)) = find_lang(auto, requested)
                && let Some(sub) = pick_subtitle(subs, preferred_format)
            {
                result.push((lang.to_owned(), sub));
            }
        }
    }

    // Sort by language for deterministic output
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

/// Generate subtitle filename matching the yt-dlp convention.
///
/// Format: `{base_stem}.{lang}.{ext}`
///
/// # Arguments
///
/// * `base_stem` - Video filename stem (without extension)
/// * `lang` - Language code (e.g., `"en"`)
/// * `ext` - Subtitle file extension (e.g., `"srt"`)
///
/// # Returns
///
/// The generated filename string.
///
/// # Examples
///
/// ```
/// use rdlp_types::subtitle_selection::subtitle_filename;
///
/// assert_eq!(subtitle_filename("My Video", "en", "srt"), "My Video.en.srt");
/// ```
#[must_use]
pub fn subtitle_filename(base_stem: &str, lang: &str, ext: &str) -> String {
    format!("{base_stem}.{lang}.{ext}")
}

/// Find a language in a subtitle map using case-insensitive matching.
fn find_lang<'a>(
    map: &'a HashMap<String, Vec<Subtitle>>,
    lang: &str,
) -> Option<(&'a str, &'a [Subtitle])> {
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(lang))
        .map(|(k, v)| (k.as_str(), v.as_slice()))
}

/// Pick the best subtitle entry from a list based on preferred format.
///
/// If `preferred_format` is set, returns the first entry matching that format.
/// Otherwise, returns the first entry in the list.
fn pick_subtitle(subs: &[Subtitle], preferred_format: Option<SubtitleFormat>) -> Option<Subtitle> {
    if let Some(pref) = preferred_format {
        let ext = pref.as_ext();
        if let Some(sub) = subs.iter().find(|s| s.ext.eq_ignore_ascii_case(ext)) {
            return Some(sub.clone());
        }
    }

    // Fallback to first available
    subs.first().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a Subtitle with the given extension.
    fn make_sub(ext: &str, lang_name: &str) -> Subtitle {
        Subtitle {
            url: format!("https://example.com/{lang_name}.{ext}"),
            ext: ext.to_string(),
            name: Some(lang_name.to_string()),
        }
    }

    /// Helper to build a subtitle map from (lang, ext) pairs.
    fn make_map(entries: &[(&str, Vec<&str>)]) -> HashMap<String, Vec<Subtitle>> {
        let mut map = HashMap::new();
        for (lang, exts) in entries {
            let subs: Vec<Subtitle> = exts.iter().map(|ext| make_sub(ext, lang)).collect();
            map.insert(lang.to_string(), subs);
        }
        map
    }

    // ===== Selection tests =====

    #[test]
    fn test_select_empty_langs_returns_empty() {
        let subs = make_map(&[("en", vec!["srt", "vtt"])]);
        let result = select_subtitles(&subs, None, &[], None, false);
        assert!(result.is_empty());
    }

    #[test]
    fn test_select_single_lang() {
        let subs = make_map(&[("en", vec!["srt", "vtt"]), ("es", vec!["srt"])]);
        let langs = vec!["en".to_string()];
        let result = select_subtitles(&subs, None, &langs, None, false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "en");
        assert_eq!(result[0].1.ext, "srt"); // first available
    }

    #[test]
    fn test_select_multiple_langs() {
        let subs = make_map(&[
            ("en", vec!["srt"]),
            ("es", vec!["vtt"]),
            ("fr", vec!["srt"]),
        ]);
        let langs = vec!["en".to_string(), "es".to_string()];
        let result = select_subtitles(&subs, None, &langs, None, false);
        assert_eq!(result.len(), 2);
        // Results are sorted by language
        assert_eq!(result[0].0, "en");
        assert_eq!(result[1].0, "es");
    }

    #[test]
    fn test_select_all_langs() {
        let subs = make_map(&[
            ("en", vec!["srt"]),
            ("es", vec!["vtt"]),
            ("fr", vec!["srt"]),
        ]);
        let langs = vec!["all".to_string()];
        let result = select_subtitles(&subs, None, &langs, None, false);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "en");
        assert_eq!(result[1].0, "es");
        assert_eq!(result[2].0, "fr");
    }

    #[test]
    fn test_select_preferred_format() {
        let subs = make_map(&[("en", vec!["vtt", "srt", "ass"])]);
        let langs = vec!["en".to_string()];
        let result = select_subtitles(&subs, None, &langs, Some(SubtitleFormat::Srt), false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1.ext, "srt");
    }

    #[test]
    fn test_select_fallback_when_preferred_unavailable() {
        let subs = make_map(&[("en", vec!["vtt", "ass"])]);
        let langs = vec!["en".to_string()];
        let result = select_subtitles(&subs, None, &langs, Some(SubtitleFormat::Srt), false);
        assert_eq!(result.len(), 1);
        // Falls back to first available since srt is not present
        assert_eq!(result[0].1.ext, "vtt");
    }

    #[test]
    fn test_select_auto_captions_when_enabled() {
        let subs = make_map(&[("en", vec!["srt"])]);
        let auto = make_map(&[("ja", vec!["vtt"])]);
        let langs = vec!["en".to_string(), "ja".to_string()];
        let result = select_subtitles(&subs, Some(&auto), &langs, None, true);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "en");
        assert_eq!(result[1].0, "ja");
    }

    #[test]
    fn test_select_manual_over_auto() {
        // Both manual and auto have "en" — manual should win
        let subs = make_map(&[("en", vec!["srt"])]);
        let auto = make_map(&[("en", vec!["vtt"])]);
        let langs = vec!["en".to_string()];
        let result = select_subtitles(&subs, Some(&auto), &langs, None, true);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "en");
        assert_eq!(result[0].1.ext, "srt"); // manual's format
    }

    #[test]
    fn test_select_missing_lang_skipped() {
        let subs = make_map(&[("en", vec!["srt"])]);
        let langs = vec!["en".to_string(), "zh".to_string()];
        let result = select_subtitles(&subs, None, &langs, None, false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "en");
    }

    #[test]
    fn test_select_auto_only_when_no_manual() {
        let subs: HashMap<String, Vec<Subtitle>> = HashMap::new();
        let auto = make_map(&[("en", vec!["vtt"])]);
        let langs = vec!["en".to_string()];
        let result = select_subtitles(&subs, Some(&auto), &langs, None, true);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "en");
        assert_eq!(result[0].1.ext, "vtt");
    }

    // ===== Filename tests =====

    #[test]
    fn test_subtitle_filename_basic() {
        assert_eq!(
            subtitle_filename("My Video", "en", "srt"),
            "My Video.en.srt"
        );
    }

    #[test]
    fn test_subtitle_filename_with_dots() {
        assert_eq!(
            subtitle_filename("My.Video.2024", "es", "vtt"),
            "My.Video.2024.es.vtt"
        );
    }
}
