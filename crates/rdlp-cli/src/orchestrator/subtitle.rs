//! Subtitle download and interactive selection orchestration
//!
//! Downloads subtitle files for a video based on config settings or
//! interactive user selection. Uses the subtitle selection logic from
//! `rdlp-types` to determine which subtitles to download based on
//! language preferences and format settings.
//!
//! The interactive multi-select menu (via `dialoguer::MultiSelect`)
//! displays available languages with format info and `[auto]` tags.

use super::{Orchestrator, Result};
use log::{debug, info, warn};
use rdlp_core::InfoDict;
use std::path::{Path, PathBuf};

/// A single item in the interactive subtitle selection menu.
///
/// Built from `InfoDict.subtitles` and `InfoDict.automatic_captions`
/// for display in the `dialoguer::MultiSelect` widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SubtitleMenuItem {
    /// Language code (e.g. "en", "es")
    pub lang: String,
    /// Human-readable display name (e.g. "English")
    pub display_name: String,
    /// Whether this entry comes from automatic captions
    pub is_auto: bool,
    /// Available subtitle format extensions (e.g. ["srt", "vtt"])
    pub formats: Vec<String>,
}

impl SubtitleMenuItem {
    /// Format this item for display in the multi-select menu.
    ///
    /// # Examples
    ///
    /// - `English (en) — srt, vtt, ass`
    /// - `Japanese (ja) [auto] — vtt`
    #[must_use]
    fn display_string(&self) -> String {
        let auto_tag = if self.is_auto { " [auto]" } else { "" };
        let fmts = self.formats.join(", ");
        format!(
            "{} ({}){} — {}",
            self.display_name, self.lang, auto_tag, fmts
        )
    }
}

/// Build subtitle menu items from an `InfoDict`.
///
/// Collects all available manual subtitles and automatic captions,
/// producing one `SubtitleMenuItem` per language. Manual subtitles
/// take precedence: if both manual and auto exist for a language,
/// only the manual entry is included.
///
/// # Arguments
/// * `info` - Video metadata containing subtitle maps
///
/// # Returns
/// Sorted vec of menu items (manual first, then auto, alphabetical by lang)
pub(super) fn build_subtitle_menu_items(info: &InfoDict) -> Vec<SubtitleMenuItem> {
    let mut items = Vec::new();
    let mut seen_langs = std::collections::HashSet::new();

    // Manual subtitles first
    if let Some(ref subs) = info.subtitles {
        for (lang, entries) in subs {
            if entries.is_empty() {
                continue;
            }
            let display_name = entries
                .iter()
                .find_map(|s| s.name.clone())
                .unwrap_or_else(|| lang.clone());
            let formats: Vec<String> = entries.iter().map(|s| s.ext.clone()).collect();

            items.push(SubtitleMenuItem {
                lang: lang.clone(),
                display_name,
                is_auto: false,
                formats,
            });
            seen_langs.insert(lang.clone());
        }
    }

    // Auto-captions (only for languages not already covered by manual)
    if let Some(ref auto) = info.automatic_captions {
        for (lang, entries) in auto {
            if entries.is_empty() || seen_langs.contains(lang) {
                continue;
            }
            let display_name = entries
                .iter()
                .find_map(|s| s.name.clone())
                .unwrap_or_else(|| lang.clone());
            let formats: Vec<String> = entries.iter().map(|s| s.ext.clone()).collect();

            items.push(SubtitleMenuItem {
                lang: lang.clone(),
                display_name,
                is_auto: true,
                formats,
            });
        }
    }

    // Sort: manual first (is_auto=false < true), then alphabetical by lang
    items.sort_by(|a, b| a.is_auto.cmp(&b.is_auto).then(a.lang.cmp(&b.lang)));
    items
}

/// Compute pre-selected indices based on config subtitle languages.
///
/// If `--sub-langs` was specified, pre-check the matching languages
/// in the menu so the user starts with them already toggled on.
///
/// # Arguments
/// * `items` - The menu items built by `build_subtitle_menu_items`
/// * `config_langs` - The `config.subtitle_langs` vec
///
/// # Returns
/// Vec of indices into `items` that should be pre-selected
pub(super) fn preselect_indices(items: &[SubtitleMenuItem], config_langs: &[String]) -> Vec<bool> {
    if config_langs.is_empty() {
        return vec![false; items.len()];
    }

    let want_all = config_langs.iter().any(|l| l.eq_ignore_ascii_case("all"));

    items
        .iter()
        .map(|item| {
            if want_all {
                true
            } else {
                config_langs
                    .iter()
                    .any(|l| l.eq_ignore_ascii_case(&item.lang))
            }
        })
        .collect()
}

impl Orchestrator {
    /// Show interactive multi-select menu for subtitle languages.
    ///
    /// Displays all available subtitles (manual + auto) with format info.
    /// Pre-checks languages matching `--sub-langs` if set.
    ///
    /// # Returns
    /// - `Ok(Some(vec))` - Selected `(lang, Subtitle)` pairs
    /// - `Ok(None)` - User cancelled (ESC)
    pub(super) async fn select_subtitles_interactive(
        &self,
        info: &InfoDict,
    ) -> Result<Option<Vec<(String, rdlp_core::Subtitle)>>> {
        let items = build_subtitle_menu_items(info);
        if items.is_empty() {
            debug!("No subtitles available for interactive selection");
            return Ok(Some(Vec::new()));
        }

        let defaults = preselect_indices(&items, &self.config.subtitle_langs);
        let display_items: Vec<String> = items.iter().map(|i| i.display_string()).collect();

        info!("Available subtitles:");

        // Run blocking dialoguer on spawn_blocking (same pattern as format selection)
        let selection = tokio::task::spawn_blocking(move || {
            use dialoguer::{MultiSelect, theme::ColorfulTheme};

            MultiSelect::with_theme(&ColorfulTheme::default())
                .with_prompt(
                    "Select subtitle languages (Space to toggle, Enter to confirm, ESC to cancel)",
                )
                .items(&display_items)
                .defaults(&defaults)
                .interact_opt()
        })
        .await
        .map_err(|e| super::OrchestratorError::Io(std::io::Error::other(e)))?
        .map_err(|e| super::OrchestratorError::Io(e.into()))?;

        let Some(selected_indices) = selection else {
            return Ok(None); // ESC pressed
        };

        if selected_indices.is_empty() {
            debug!("No subtitles selected");
            return Ok(Some(Vec::new()));
        }

        // Map selected indices back to (lang, best_subtitle) pairs
        let mut result = Vec::new();
        for &idx in &selected_indices {
            let item = &items[idx];
            // Find the best subtitle entry for this language
            if let Some(sub) = self.pick_best_subtitle_for_lang(info, &item.lang, item.is_auto) {
                result.push((item.lang.clone(), sub));
            }
        }

        info!(
            "Selected {} subtitle language(s): {}",
            result.len(),
            result
                .iter()
                .map(|(l, _)| l.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );

        Ok(Some(result))
    }

    /// Decide whether to show interactive subtitle menu or use config.
    ///
    /// Shows the menu when:
    /// - `interactive` is true and subtitles are available
    /// - `list_subs` is true
    ///
    /// Otherwise returns an empty selection (subtitles will be handled
    /// by the config-based path in `download_subtitles`).
    ///
    /// # Returns
    /// - `Ok(Some(vec))` - Selected subtitles (may be empty)
    /// - `Ok(None)` - User cancelled
    pub(super) async fn select_subtitles_if_needed(
        &self,
        info: &InfoDict,
        interactive: bool,
        list_subs: bool,
    ) -> Result<Option<Vec<(String, rdlp_core::Subtitle)>>> {
        let has_subs = info.subtitles.as_ref().is_some_and(|s| !s.is_empty())
            || info
                .automatic_captions
                .as_ref()
                .is_some_and(|a| !a.is_empty());

        if (interactive || list_subs) && has_subs {
            self.select_subtitles_interactive(info).await
        } else {
            Ok(Some(Vec::new()))
        }
    }

    /// Pick the best subtitle entry for a language from the info dict.
    ///
    /// Respects `config.subtitle_format` preference, falling back to
    /// the first available entry.
    fn pick_best_subtitle_for_lang(
        &self,
        info: &InfoDict,
        lang: &str,
        is_auto: bool,
    ) -> Option<rdlp_core::Subtitle> {
        let source = if is_auto {
            info.automatic_captions.as_ref()
        } else {
            info.subtitles.as_ref()
        };

        let entries = source?.get(lang)?;
        if entries.is_empty() {
            return None;
        }

        // Try preferred format first
        if let Some(fmt) = self.config.subtitle_format {
            let ext = fmt.as_ext();
            if let Some(sub) = entries.iter().find(|s| s.ext.eq_ignore_ascii_case(ext)) {
                return Some(sub.clone());
            }
        }

        // Fallback to first entry
        Some(entries[0].clone())
    }

    /// Resolve subtitle entries for a specific episode from language names.
    ///
    /// Used by playlist downloads: the user selects languages once (from the
    /// first episode's menu), then each episode resolves its own subtitle
    /// URLs for those languages.
    ///
    /// # Arguments
    /// * `info` - Episode metadata with subtitle URLs
    /// * `langs` - Language names selected by the user
    ///
    /// # Returns
    /// Vec of (lang, Subtitle) pairs with URLs specific to this episode
    pub(super) fn resolve_subtitles_for_episode(
        &self,
        info: &InfoDict,
        langs: &[String],
    ) -> Vec<(String, rdlp_core::Subtitle)> {
        let mut result = Vec::new();
        for lang in langs {
            // Try manual subtitles first, then auto-captions
            if let Some(sub) = self.pick_best_subtitle_for_lang(info, lang, false) {
                result.push((lang.clone(), sub));
            } else if let Some(sub) = self.pick_best_subtitle_for_lang(info, lang, true) {
                result.push((lang.clone(), sub));
            } else {
                debug!(lang:%; "No subtitle found for language in this episode");
            }
        }
        result
    }

    /// Download subtitles for a video.
    ///
    /// Uses pre-selected subtitles from interactive menu if provided,
    /// otherwise falls back to config-based selection.
    ///
    /// # Arguments
    /// * `info` - Video metadata with subtitle URLs
    /// * `output_path` - Path to the downloaded video file
    /// * `interactive_selection` - Pre-selected subtitles from interactive menu
    ///
    /// # Returns
    /// Vec of (language, path) for downloaded subtitle files
    pub(super) async fn download_subtitles(
        &self,
        info: &InfoDict,
        output_path: &Path,
        interactive_selection: &[(String, rdlp_core::Subtitle)],
    ) -> Result<Vec<(String, PathBuf)>> {
        // If interactive selection is non-empty, use it directly
        if !interactive_selection.is_empty() {
            return self
                .download_subtitles_from_selection(interactive_selection, output_path)
                .await;
        }

        // Config-based path: delegate to the structured pipeline
        if !self.config.write_subtitles && !self.config.embed_subtitles {
            return Ok(Vec::new());
        }

        let (downloaded, _warnings) = self
            .download_subtitles_with_pipeline(info, output_path)
            .await?;
        Ok(downloaded)
    }

    /// Download pre-selected subtitles (from interactive menu).
    ///
    /// # Arguments
    /// * `selected` - List of (lang, Subtitle) pairs from interactive selection
    /// * `output_path` - Path to the video file (subtitle paths derived from it)
    ///
    /// # Returns
    /// Vec of (language, path) for downloaded subtitle files
    pub(super) async fn download_subtitles_from_selection(
        &self,
        selected: &[(String, rdlp_core::Subtitle)],
        output_path: &Path,
    ) -> Result<Vec<(String, PathBuf)>> {
        let stem = output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video");
        let parent = output_path.parent().unwrap_or(Path::new("."));

        let mut downloaded = Vec::new();

        for (lang, sub) in selected {
            let sub_filename = format!("{stem}.{lang}.{}", sub.ext);
            let sub_path = parent.join(&sub_filename);

            info!("Downloading subtitle: lang={lang}, url={}", sub.url);

            match self.download_subtitle_file(&sub.url, &sub_path).await {
                Ok(()) => {
                    info!("Subtitle downloaded: {}", sub_path.display());
                    downloaded.push((lang.clone(), sub_path));
                }
                Err(e) => {
                    warn!("Failed to download subtitle for {lang}: {e}");
                }
            }
        }

        Ok(downloaded)
    }

    /// Download only subtitles (no video) for `--list-subs-only` mode.
    ///
    /// Extracts metadata, shows interactive subtitle selection, downloads
    /// selected subtitle files, and returns their paths.
    ///
    /// # Arguments
    /// * `info` - Pre-extracted video metadata
    ///
    /// # Returns
    /// - `Ok(Some(paths))` - Downloaded subtitle file paths
    /// - `Ok(None)` - User cancelled
    pub(super) async fn download_subtitles_standalone(
        &self,
        info: &InfoDict,
    ) -> Result<Option<Vec<PathBuf>>> {
        let Some(selected) = self.select_subtitles_interactive(info).await? else {
            return Ok(None);
        };

        if selected.is_empty() {
            info!("No subtitles selected");
            return Ok(Some(Vec::new()));
        }

        // Generate output path from video title (no format needed)
        let sanitized = self.sanitize_filename(&info.title);
        let output_stub = self
            .config
            .output_directory
            .join(format!("{sanitized}.mp4"));

        let downloaded = self
            .download_subtitles_from_selection(&selected, &output_stub)
            .await?;

        let paths: Vec<PathBuf> = downloaded.into_iter().map(|(_, p)| p).collect();
        Ok(Some(paths))
    }

    /// Download subtitles using the structured pipeline with status reporting.
    ///
    /// Normalizes InfoDict subtitles into [`SubtitleResult`], optionally
    /// validates URLs, applies policy (language filtering, strict mode),
    /// and downloads. Returns downloaded paths and any warnings.
    pub(super) async fn download_subtitles_with_pipeline(
        &self,
        info: &InfoDict,
        output_path: &Path,
    ) -> Result<(Vec<(String, PathBuf)>, Vec<String>)> {
        use super::subtitle_pipeline::{
            apply_subtitle_policy, normalize_subtitles, validate_subtitle_urls,
        };

        // Stage 3: Normalize
        let mut result = normalize_subtitles(info);

        // Stage 2: Validate (optional, only if verify_sub_urls is true)
        if self.config.verify_sub_urls && result.has_tracks() {
            result = validate_subtitle_urls(result, &self.extraction_context.http_client).await;
        }

        // Stage 4: Policy
        let outcome = apply_subtitle_policy(
            &result,
            &self.config.subtitle_langs,
            self.config.subtitle_format,
            self.config.write_auto_subtitles,
            self.config.strict_subs,
        );

        // Log warnings
        for warning in &outcome.warnings {
            warn!("{warning}");
        }

        // Hard error in strict mode
        if outcome.should_fail {
            let msg = outcome
                .error_message
                .unwrap_or_else(|| "Subtitle policy check failed".to_string());
            return Err(super::OrchestratorError::DownloadFailed(
                rdlp_core::RdlpError::Extraction(msg),
            ));
        }

        // Download selected tracks
        let stem = output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("video");
        let parent = output_path.parent().unwrap_or(Path::new("."));
        let mut downloaded = Vec::new();

        for track in &outcome.selected {
            let sub_filename = format!("{stem}.{}.{}", track.language, track.ext);
            let sub_path = parent.join(&sub_filename);

            info!(
                lang:% = track.language,
                ext:% = track.ext;
                "Downloading subtitle"
            );

            match self.download_subtitle_file(&track.url, &sub_path).await {
                Ok(()) => {
                    info!("Subtitle downloaded: {}", sub_path.display());
                    downloaded.push((track.language.clone(), sub_path));
                }
                Err(e) => {
                    warn!(
                        lang:% = track.language;
                        "Failed to download subtitle: {e}"
                    );
                }
            }
        }

        let warnings = outcome.warnings;
        Ok((downloaded, warnings))
    }

    /// Download a single subtitle file via HTTP.
    async fn download_subtitle_file(
        &self,
        url: &str,
        output: &Path,
    ) -> std::result::Result<(), anyhow::Error> {
        let response = self.extraction_context.http_client.get(url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("Subtitle download failed with status {}", response.status());
        }

        let bytes = response.bytes().await?;
        tokio::fs::write(output, &bytes).await?;
        Ok(())
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
#[cfg(test)]
fn select_subtitles_for_download(
    subtitles: &std::collections::HashMap<String, Vec<rdlp_core::Subtitle>>,
    auto_captions: &std::collections::HashMap<String, Vec<rdlp_core::Subtitle>>,
    requested_langs: &[String],
    preferred_format: Option<rdlp_core::SubtitleFormat>,
    include_auto: bool,
) -> Vec<(String, String, String)> {
    let mut result = Vec::new();
    let want_all = requested_langs.is_empty()
        || requested_langs
            .iter()
            .any(|l| l.eq_ignore_ascii_case("all"));

    // Helper: pick best subtitle entry for a language
    let pick_best = |entries: &[rdlp_core::Subtitle]| -> Option<(String, String)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_core::{Subtitle, SubtitleFormat};
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
        assert_eq!(display, "Japanese (ja) [auto] — vtt");
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
        assert_eq!(display, "English (en) — srt, vtt");
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

        // Only one "en" entry — the manual one
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
        let selected_indices = vec![0, 2];

        let selected_langs: Vec<&str> = selected_indices
            .iter()
            .map(|&i| items[i].lang.as_str())
            .collect();

        assert_eq!(selected_langs, vec!["en", "fr"]);
    }

    // ===== Existing select_subtitles_for_download tests =====

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

        let result = select_subtitles_for_download(
            &subs,
            &HashMap::new(),
            &["all".to_string()],
            None,
            false,
        );

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
}
