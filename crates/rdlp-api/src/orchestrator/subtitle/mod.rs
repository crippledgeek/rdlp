//! Subtitle download and interactive selection orchestration
//!
//! Downloads subtitle files for a video based on config settings or
//! interactive user selection. Uses the subtitle selection logic from
//! `rdlp-types` to determine which subtitles to download based on
//! language preferences and format settings.
//!
//! The interactive multi-select is delegated to the frontend-provided
//! [`InteractiveCallback`] and displays available languages with
//! format info and `[auto]` tags.

mod download;

use super::{Orchestrator, Result};
use log::{debug, info};
use rdlp_core::InfoDict;

#[cfg(test)]
mod tests_menu;
#[cfg(test)]
mod tests_selection;

/// A single item in the interactive subtitle selection menu.
///
/// Built from `InfoDict.subtitles` and `InfoDict.automatic_captions`
/// for display in the interactive subtitle selection UI.
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
    /// - `English (en) -- srt, vtt, ass`
    /// - `Japanese (ja) [auto] -- vtt`
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
                config_langs.iter().any(|l| {
                    item.lang.eq_ignore_ascii_case(l)
                        || (l.len() <= 3
                            && item
                                .lang
                                .to_ascii_lowercase()
                                .starts_with(&l.to_ascii_lowercase()))
                })
            }
        })
        .collect()
}

impl Orchestrator {
    /// Show interactive multi-select menu for subtitle languages.
    ///
    /// Delegates to [`InteractiveCallback::select_subtitles`] if available.
    /// Pre-checks languages matching `--sub-langs` if set.
    ///
    /// # Returns
    /// - `Ok(Some(vec))` - Selected `(lang, Subtitle)` pairs
    /// - `Ok(None)` - User cancelled
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

        // Delegate to the interactive callback
        let Some(callback) = self.interactive.as_ref() else {
            // No interactive callback -- return empty selection
            debug!("No interactive callback, skipping subtitle selection");
            return Ok(Some(Vec::new()));
        };

        let selection = callback.select_subtitles(&display_items, &defaults).await;

        let Some(selected_indices) = selection else {
            return Ok(None); // User cancelled
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

        let map = source?;

        // Try exact key first, then fuzzy (ISO prefix) match on keys.
        // 9anime uses labels ("English") as keys while --sub-langs uses
        // codes ("en"), so we need prefix matching.
        let entries = map.get(lang).or_else(|| {
            map.iter()
                .find(|(key, _)| {
                    key.eq_ignore_ascii_case(lang)
                        || (lang.len() <= 3
                            && key
                                .to_ascii_lowercase()
                                .starts_with(&lang.to_ascii_lowercase()))
                })
                .map(|(_, v)| v)
        })?;

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
}
