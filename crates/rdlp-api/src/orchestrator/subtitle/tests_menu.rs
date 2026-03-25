//! Tests for subtitle menu building, display, and preselection

use super::*;
use rdlp_types::{InfoDict, Subtitle};
use std::collections::HashMap;

fn make_sub(url: &str, ext: &str) -> Subtitle {
    Subtitle {
        url: url.to_string(),
        ext: ext.to_string(),
        name: None,
    }
}

fn make_named_sub(url: &str, ext: &str, name: &str) -> Subtitle {
    Subtitle {
        url: url.to_string(),
        ext: ext.to_string(),
        name: Some(name.to_string()),
    }
}

// ===== build_subtitle_menu_items tests =====

#[test]
fn test_build_subtitle_menu_items_basic() {
    let mut info = InfoDict::new("id", "title", "test", "http://example.com");
    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![
            make_named_sub("http://ex.com/en.srt", "srt", "English"),
            make_named_sub("http://ex.com/en.vtt", "vtt", "English"),
        ],
    );
    subs.insert(
        "es".to_string(),
        vec![make_named_sub("http://ex.com/es.srt", "srt", "Spanish")],
    );
    info.subtitles = Some(subs);

    let items = build_subtitle_menu_items(&info);

    assert_eq!(items.len(), 2);
    // Sorted by lang alphabetically
    assert_eq!(items[0].lang, "en");
    assert_eq!(items[0].display_name, "English");
    assert!(!items[0].is_auto);
    assert_eq!(items[0].formats, vec!["srt", "vtt"]);

    assert_eq!(items[1].lang, "es");
    assert_eq!(items[1].display_name, "Spanish");
    assert!(!items[1].is_auto);
}

#[test]
fn test_build_subtitle_menu_items_with_auto() {
    let mut info = InfoDict::new("id", "title", "test", "http://example.com");

    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_named_sub("http://ex.com/en.srt", "srt", "English")],
    );
    info.subtitles = Some(subs);

    let mut auto = HashMap::new();
    auto.insert(
        "ja".to_string(),
        vec![make_named_sub("http://ex.com/ja.vtt", "vtt", "Japanese")],
    );
    info.automatic_captions = Some(auto);

    let items = build_subtitle_menu_items(&info);

    assert_eq!(items.len(), 2);
    // Manual first, then auto
    assert_eq!(items[0].lang, "en");
    assert!(!items[0].is_auto);
    assert_eq!(items[1].lang, "ja");
    assert!(items[1].is_auto);
}

#[test]
fn test_build_subtitle_menu_items_auto_tag_in_display() {
    let item = SubtitleMenuItem {
        lang: "ja".to_string(),
        display_name: "Japanese".to_string(),
        is_auto: true,
        formats: vec!["vtt".to_string()],
    };

    let display = item.display_string();
    assert!(display.contains("[auto]"));
    assert_eq!(display, "Japanese (ja) [auto] \u{2014} vtt");
}

#[test]
fn test_build_subtitle_menu_items_no_auto_tag_for_manual() {
    let item = SubtitleMenuItem {
        lang: "en".to_string(),
        display_name: "English".to_string(),
        is_auto: false,
        formats: vec!["srt".to_string(), "vtt".to_string()],
    };

    let display = item.display_string();
    assert!(!display.contains("[auto]"));
    assert_eq!(display, "English (en) \u{2014} srt, vtt");
}

#[test]
fn test_build_subtitle_menu_items_empty() {
    let info = InfoDict::new("id", "title", "test", "http://example.com");
    let items = build_subtitle_menu_items(&info);
    assert!(items.is_empty());
}

#[test]
fn test_build_subtitle_menu_items_manual_overrides_auto() {
    let mut info = InfoDict::new("id", "title", "test", "http://example.com");

    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_named_sub("http://ex.com/en.srt", "srt", "English")],
    );
    info.subtitles = Some(subs);

    let mut auto = HashMap::new();
    auto.insert(
        "en".to_string(),
        vec![make_named_sub(
            "http://ex.com/auto-en.vtt",
            "vtt",
            "English",
        )],
    );
    info.automatic_captions = Some(auto);

    let items = build_subtitle_menu_items(&info);

    // Only one "en" entry -- the manual one
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].lang, "en");
    assert!(!items[0].is_auto);
}

#[test]
fn test_build_subtitle_menu_items_fallback_name_to_lang() {
    let mut info = InfoDict::new("id", "title", "test", "http://example.com");
    let mut subs = HashMap::new();
    subs.insert(
        "fr".to_string(),
        vec![make_sub("http://ex.com/fr.srt", "srt")], // no name
    );
    info.subtitles = Some(subs);

    let items = build_subtitle_menu_items(&info);

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].display_name, "fr"); // Fallback to lang code
}

// ===== preselect_indices tests =====

#[test]
fn test_preselect_from_config_langs() {
    let items = vec![
        SubtitleMenuItem {
            lang: "en".to_string(),
            display_name: "English".to_string(),
            is_auto: false,
            formats: vec!["srt".to_string()],
        },
        SubtitleMenuItem {
            lang: "es".to_string(),
            display_name: "Spanish".to_string(),
            is_auto: false,
            formats: vec!["srt".to_string()],
        },
        SubtitleMenuItem {
            lang: "fr".to_string(),
            display_name: "French".to_string(),
            is_auto: false,
            formats: vec!["srt".to_string()],
        },
    ];

    let defaults = preselect_indices(&items, &["en".to_string(), "fr".to_string()]);

    assert_eq!(defaults, vec![true, false, true]);
}

#[test]
fn test_preselect_empty_config_langs() {
    let items = vec![SubtitleMenuItem {
        lang: "en".to_string(),
        display_name: "English".to_string(),
        is_auto: false,
        formats: vec!["srt".to_string()],
    }];

    let defaults = preselect_indices(&items, &[]);

    assert_eq!(defaults, vec![false]);
}

#[test]
fn test_preselect_all_keyword() {
    let items = vec![
        SubtitleMenuItem {
            lang: "en".to_string(),
            display_name: "English".to_string(),
            is_auto: false,
            formats: vec!["srt".to_string()],
        },
        SubtitleMenuItem {
            lang: "es".to_string(),
            display_name: "Spanish".to_string(),
            is_auto: false,
            formats: vec!["srt".to_string()],
        },
    ];

    let defaults = preselect_indices(&items, &["all".to_string()]);

    assert_eq!(defaults, vec![true, true]);
}

#[test]
fn test_preselect_case_insensitive() {
    let items = vec![SubtitleMenuItem {
        lang: "EN".to_string(),
        display_name: "English".to_string(),
        is_auto: false,
        formats: vec!["srt".to_string()],
    }];

    let defaults = preselect_indices(&items, &["en".to_string()]);

    assert_eq!(defaults, vec![true]);
}

// ===== subtitle_selection_integration test =====

#[test]
fn test_subtitle_selection_integration() {
    // Verify that selected indices correctly map to (lang, subtitle) pairs
    let mut info = InfoDict::new("id", "title", "test", "http://example.com");

    let mut subs = HashMap::new();
    subs.insert(
        "en".to_string(),
        vec![make_named_sub("http://ex.com/en.srt", "srt", "English")],
    );
    subs.insert(
        "es".to_string(),
        vec![make_named_sub("http://ex.com/es.vtt", "vtt", "Spanish")],
    );
    subs.insert(
        "fr".to_string(),
        vec![make_named_sub("http://ex.com/fr.srt", "srt", "French")],
    );
    info.subtitles = Some(subs);

    let items = build_subtitle_menu_items(&info);

    // Simulate selecting indices 0 and 2 (en and fr, sorted alphabetically)
    let selected_indices = [0, 2];

    let selected_langs: Vec<&str> = selected_indices
        .iter()
        .map(|&i| items[i].lang.as_str())
        .collect();

    assert_eq!(selected_langs, vec!["en", "fr"]);
}
