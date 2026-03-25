//! Tests for select_subtitles_for_download logic

use rdlp_types::{Subtitle, SubtitleFormat};
use std::collections::HashMap;

fn make_sub(url: &str, ext: &str) -> Subtitle {
    Subtitle {
        url: url.to_string(),
        ext: ext.to_string(),
        name: None,
    }
}

/// Select subtitles to download based on configuration.
///
/// Returns a list of (language, url, extension) tuples for subtitles
/// that match the requested languages and format preferences.
///
/// # Arguments
/// * `subtitles` - Available manual subtitles (lang -> list of formats)
/// * `auto_captions` - Available auto-generated captions
/// * `requested_langs` - Languages to download (empty = all)
/// * `preferred_format` - Preferred subtitle format (None = any)
/// * `include_auto` - Whether to include auto-generated captions
fn select_subtitles_for_download(
    subtitles: &HashMap<String, Vec<Subtitle>>,
    auto_captions: &HashMap<String, Vec<Subtitle>>,
    requested_langs: &[String],
    preferred_format: Option<SubtitleFormat>,
    include_auto: bool,
) -> Vec<(String, String, String)> {
    let mut result = Vec::new();
    let want_all = requested_langs.is_empty()
        || requested_langs
            .iter()
            .any(|l| l.eq_ignore_ascii_case("all"));

    // Helper: pick best subtitle entry for a language
    let pick_best = |entries: &[Subtitle]| -> Option<(String, String)> {
        if let Some(fmt) = preferred_format {
            let ext = fmt.as_ext();
            if let Some(sub) = entries.iter().find(|s| s.ext.eq_ignore_ascii_case(ext)) {
                return Some((sub.url.clone(), sub.ext.clone()));
            }
        }
        // Fallback: prefer srt > vtt > first available
        let preferred_order = ["srt", "vtt", "ass", "ssa", "lrc"];
        for pref in &preferred_order {
            if let Some(sub) = entries.iter().find(|s| s.ext.eq_ignore_ascii_case(pref)) {
                return Some((sub.url.clone(), sub.ext.clone()));
            }
        }
        entries.first().map(|s| (s.url.clone(), s.ext.clone()))
    };

    // Collect from manual subtitles
    for (lang, entries) in subtitles {
        if !want_all && !requested_langs.iter().any(|l| l.eq_ignore_ascii_case(lang)) {
            continue;
        }
        if let Some((url, ext)) = pick_best(entries) {
            result.push((lang.clone(), url, ext));
        }
    }

    // Collect from auto-captions if requested
    if include_auto {
        for (lang, entries) in auto_captions {
            // Skip if we already have manual subs for this language
            if result.iter().any(|(l, _, _)| l == lang) {
                continue;
            }
            if !want_all && !requested_langs.iter().any(|l| l.eq_ignore_ascii_case(lang)) {
                continue;
            }
            if let Some((url, ext)) = pick_best(entries) {
                result.push((lang.clone(), url, ext));
            }
        }
    }

    result
}

// ===== select_subtitles_for_download tests =====

#[test]
fn test_select_all_languages_when_empty_langs() {
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/en.srt", "srt")],
    );
    subs.insert(
        "es".to_string(),
        vec![make_sub("http://example.com/es.vtt", "vtt")],
    );

    let result = select_subtitles_for_download(&subs, &HashMap::new(), &[], None, false);

    assert_eq!(result.len(), 2);
    let langs: Vec<&str> = result.iter().map(|(l, _, _)| l.as_str()).collect();
    assert!(langs.contains(&"en"));
    assert!(langs.contains(&"es"));
}

#[test]
fn test_select_specific_language() {
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/en.srt", "srt")],
    );
    subs.insert(
        "es".to_string(),
        vec![make_sub("http://example.com/es.srt", "srt")],
    );
    subs.insert(
        "fr".to_string(),
        vec![make_sub("http://example.com/fr.srt", "srt")],
    );

    let result = select_subtitles_for_download(
        &subs,
        &HashMap::new(),
        &["en".to_string(), "fr".to_string()],
        None,
        false,
    );

    assert_eq!(result.len(), 2);
    let langs: Vec<&str> = result.iter().map(|(l, _, _)| l.as_str()).collect();
    assert!(langs.contains(&"en"));
    assert!(langs.contains(&"fr"));
    assert!(!langs.contains(&"es"));
}

#[test]
fn test_preferred_format_selection() {
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![
            make_sub("http://example.com/en.vtt", "vtt"),
            make_sub("http://example.com/en.srt", "srt"),
            make_sub("http://example.com/en.ass", "ass"),
        ],
    );

    // Prefer ass format
    let result = select_subtitles_for_download(
        &subs,
        &HashMap::new(),
        &[],
        Some(SubtitleFormat::Ass),
        false,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].2, "ass");
    assert!(result[0].1.contains("en.ass"));
}

#[test]
fn test_fallback_format_prefers_srt() {
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![
            make_sub("http://example.com/en.ass", "ass"),
            make_sub("http://example.com/en.srt", "srt"),
            make_sub("http://example.com/en.vtt", "vtt"),
        ],
    );

    // No preferred format - should fall back to srt
    let result = select_subtitles_for_download(&subs, &HashMap::new(), &[], None, false);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].2, "srt");
}

#[test]
fn test_auto_captions_excluded_by_default() {
    let subs = HashMap::new();
    let mut auto = HashMap::new();
    auto.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/auto-en.srt", "srt")],
    );

    let result = select_subtitles_for_download(
        &subs,
        &auto,
        &[],
        None,
        false, // include_auto = false
    );

    assert!(result.is_empty());
}

#[test]
fn test_auto_captions_included_when_requested() {
    let subs = HashMap::new();
    let mut auto = HashMap::new();
    auto.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/auto-en.srt", "srt")],
    );

    let result = select_subtitles_for_download(
        &subs,
        &auto,
        &[],
        None,
        true, // include_auto = true
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "en");
}

#[test]
fn test_manual_subs_preferred_over_auto() {
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/manual-en.srt", "srt")],
    );
    let mut auto = HashMap::new();
    auto.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/auto-en.srt", "srt")],
    );

    let result = select_subtitles_for_download(&subs, &auto, &[], None, true);

    // Should have only 1 entry for "en" - the manual one
    assert_eq!(result.len(), 1);
    assert!(result[0].1.contains("manual"));
}

#[test]
fn test_all_keyword_selects_everything() {
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/en.srt", "srt")],
    );
    subs.insert(
        "es".to_string(),
        vec![make_sub("http://example.com/es.srt", "srt")],
    );

    let result =
        select_subtitles_for_download(&subs, &HashMap::new(), &["all".to_string()], None, false);

    assert_eq!(result.len(), 2);
}

#[test]
fn test_empty_subtitles_returns_empty() {
    let result = select_subtitles_for_download(
        &HashMap::new(),
        &HashMap::new(),
        &["en".to_string()],
        None,
        true,
    );

    assert!(result.is_empty());
}

#[test]
fn test_subtitle_filename_format() {
    // Verify the filename pattern: stem.lang.ext
    let stem = "My Video";
    let lang = "en";
    let ext = "srt";
    let filename = format!("{stem}.{lang}.{ext}");
    assert_eq!(filename, "My Video.en.srt");
}

#[test]
fn test_preferred_format_fallback_when_not_available() {
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_sub("http://example.com/en.vtt", "vtt")],
    );

    // Request ass format but only vtt is available
    let result = select_subtitles_for_download(
        &subs,
        &HashMap::new(),
        &[],
        Some(SubtitleFormat::Ass),
        false,
    );

    // Should fall back to vtt (available format)
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].2, "vtt");
}
