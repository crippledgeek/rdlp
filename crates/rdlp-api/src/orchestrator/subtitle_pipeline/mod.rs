//! Subtitle pipeline: validation, normalization, and policy application
//!
//! Implements stages 2-4 of the subtitle pipeline:
//! 1. **Extractor** -- site-specific, handled by extractors (not this module)
//! 2. **Validator** -- HEAD/GET URL pre-checks ([`validate_subtitle_urls`])
//! 3. **Normalizer** -- InfoDict -> SubtitleResult ([`normalize_subtitles`])
//! 4. **Policy** -- apply user flags, decide warn vs error ([`apply_subtitle_policy`])

use log::{debug, warn};
use rdlp_types::{
    InfoDict, SubtitleDiagnostic, SubtitleFormat, SubtitleReason, SubtitleResult, SubtitleStatus,
    SubtitleTrack, normalize_from_info_dict,
};
use std::time::Duration;

#[cfg(test)]
mod tests;

// --- Stage 2: Validator ---

const VALIDATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Validate subtitle URLs via HEAD requests.
///
/// Only called when `config.verify_sub_urls` is true. Filters out
/// tracks with unreachable URLs and records reasons/diagnostics.
pub(super) async fn validate_subtitle_urls(
    result: SubtitleResult,
    client: &wreq::Client,
) -> SubtitleResult {
    let mut valid_tracks = Vec::new();
    let mut reasons = result.reasons;
    let mut diagnostics = result.diagnostics;

    for track in result.tracks {
        match validate_single_url(&track.url, client).await {
            UrlValidation::Reachable => {
                valid_tracks.push(track);
            }
            UrlValidation::Unreachable(status) => {
                warn!(
                    lang:% = track.language,
                    status;
                    "Subtitle URL unreachable"
                );
                if status == 401 || status == 403 {
                    reasons.push(SubtitleReason::RequiresAuth);
                } else {
                    reasons.push(SubtitleReason::UrlNotReachable(status));
                }
                diagnostics.push(SubtitleDiagnostic {
                    key: format!("unreachable_{}", track.language),
                    value: format!("HTTP {status}: {}", track.url),
                });
            }
            UrlValidation::NetworkError(e) => {
                warn!(
                    lang:% = track.language;
                    "Subtitle URL validation error: {e}"
                );
                diagnostics.push(SubtitleDiagnostic {
                    key: format!("validation_error_{}", track.language),
                    value: e,
                });
            }
        }
    }

    let status = if valid_tracks.is_empty() && !reasons.is_empty() {
        SubtitleStatus::ErrorSoft
    } else if valid_tracks.is_empty() {
        SubtitleStatus::NotAvailable
    } else {
        SubtitleStatus::Available
    };

    SubtitleResult {
        tracks: valid_tracks,
        status,
        reasons,
        diagnostics,
    }
}

enum UrlValidation {
    Reachable,
    Unreachable(u16),
    NetworkError(String),
}

async fn validate_single_url(url: &str, client: &wreq::Client) -> UrlValidation {
    // Try HEAD first
    match client.head(url).timeout(VALIDATION_TIMEOUT).send().await {
        Ok(resp) if resp.status().is_success() => return UrlValidation::Reachable,
        Ok(resp) if resp.status().as_u16() == 405 => {
            // HEAD not allowed -- fallback to GET with range
            debug!("HEAD 405 for subtitle URL, falling back to ranged GET");
        }
        Ok(resp) => return UrlValidation::Unreachable(resp.status().as_u16()),
        Err(e) => return UrlValidation::NetworkError(e.to_string()),
    }

    // Fallback: GET with range header (0-1KB)
    match client
        .get(url)
        .header("Range", "bytes=0-1023")
        .timeout(VALIDATION_TIMEOUT)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 206 => {
            UrlValidation::Reachable
        }
        Ok(resp) => UrlValidation::Unreachable(resp.status().as_u16()),
        Err(e) => UrlValidation::NetworkError(e.to_string()),
    }
}

// --- Stage 3: Normalizer ---

/// Normalize InfoDict subtitle maps into a [`SubtitleResult`].
///
/// Converts the legacy `HashMap<String, Vec<Subtitle>>` fields into
/// [`SubtitleTrack`] entries with proper status and reason inference.
pub(super) fn normalize_subtitles(info: &InfoDict) -> SubtitleResult {
    normalize_from_info_dict(info.subtitles.as_ref(), info.automatic_captions.as_ref())
}

// --- Stage 4: Policy ---

/// Outcome of subtitle policy evaluation.
#[derive(Debug)]
pub(super) struct SubtitlePolicyOutcome {
    /// Tracks selected for download
    pub selected: Vec<SubtitleTrack>,
    /// Warning messages to display (non-fatal)
    pub warnings: Vec<String>,
    /// Whether the download should be aborted
    pub should_fail: bool,
    /// Error message if should_fail is true
    pub error_message: Option<String>,
}

/// Apply subtitle policy based on user config.
///
/// Filters tracks by requested languages, format preference,
/// and auto-caption inclusion. Decides whether missing subtitles
/// should warn or error based on `strict_subs`.
pub(super) fn apply_subtitle_policy(
    result: &SubtitleResult,
    requested_langs: &[String],
    preferred_format: Option<SubtitleFormat>,
    include_auto: bool,
    strict_subs: bool,
) -> SubtitlePolicyOutcome {
    let mut selected = Vec::new();
    let mut warnings = Vec::new();

    // No languages requested -> noop
    if requested_langs.is_empty() {
        return SubtitlePolicyOutcome {
            selected,
            warnings,
            should_fail: false,
            error_message: None,
        };
    }

    let want_all = requested_langs
        .iter()
        .any(|l| l.eq_ignore_ascii_case("all"));

    if want_all {
        for track in &result.tracks {
            if !include_auto && track.is_auto {
                continue;
            }
            selected.push(track.clone());
        }
        // Deduplicate: keep best format per language
        selected = deduplicate_by_language(selected, preferred_format);
    } else {
        for lang in requested_langs {
            // Try manual first
            let manual: Vec<&SubtitleTrack> = result
                .tracks
                .iter()
                .filter(|t| !t.is_auto && track_matches_lang(t, lang))
                .collect();

            if let Some(track) = pick_best_track(&manual, preferred_format) {
                selected.push(track);
                continue;
            }

            // Fallback to auto if included
            if include_auto {
                let auto: Vec<&SubtitleTrack> = result
                    .tracks
                    .iter()
                    .filter(|t| t.is_auto && track_matches_lang(t, lang))
                    .collect();

                if let Some(track) = pick_best_track(&auto, preferred_format) {
                    selected.push(track);
                    continue;
                }
            }

            warnings.push(format!(
                "Requested subtitle language '{lang}' not available"
            ));
        }
    }

    let should_fail = strict_subs && !warnings.is_empty();
    let error_message = if should_fail {
        Some(format!(
            "Strict subtitle mode: {} language(s) unavailable",
            warnings.len()
        ))
    } else {
        None
    };

    SubtitlePolicyOutcome {
        selected,
        warnings,
        should_fail,
        error_message,
    }
}

/// Check whether a subtitle track matches a requested language.
///
/// Supports exact match (`"English"` == `"English"`), ISO code prefix
/// (`"en"` matches `"English"`), and name field fallback.
fn track_matches_lang(track: &SubtitleTrack, lang: &str) -> bool {
    // Exact case-insensitive match on the language key
    if track.language.eq_ignore_ascii_case(lang) {
        return true;
    }
    // ISO code prefix: "en" starts "English", "ja" starts "Japanese"
    if lang.len() <= 3
        && track
            .language
            .to_ascii_lowercase()
            .starts_with(&lang.to_ascii_lowercase())
    {
        return true;
    }
    // Check the optional name field (e.g., name: Some("English"))
    if let Some(ref name) = track.name {
        if name.eq_ignore_ascii_case(lang) {
            return true;
        }
        if lang.len() <= 3
            && name
                .to_ascii_lowercase()
                .starts_with(&lang.to_ascii_lowercase())
        {
            return true;
        }
    }
    false
}

/// Pick the best track from candidates based on format preference.
fn pick_best_track(
    candidates: &[&SubtitleTrack],
    preferred_format: Option<SubtitleFormat>,
) -> Option<SubtitleTrack> {
    if candidates.is_empty() {
        return None;
    }

    // Prefer user-requested format
    if let Some(pref) = preferred_format {
        let ext = pref.as_ext();
        if let Some(track) = candidates.iter().find(|t| t.ext.eq_ignore_ascii_case(ext)) {
            return Some((*track).clone());
        }
    }

    // Fallback order: srt > vtt > ass > ssa > lrc > first
    const FALLBACK_ORDER: &[&str] = &["srt", "vtt", "ass", "ssa", "lrc"];
    for pref in FALLBACK_ORDER {
        if let Some(track) = candidates.iter().find(|t| t.ext.eq_ignore_ascii_case(pref)) {
            return Some((*track).clone());
        }
    }

    Some(candidates[0].clone())
}

/// Keep only one track per language (best format wins).
fn deduplicate_by_language(
    tracks: Vec<SubtitleTrack>,
    preferred_format: Option<SubtitleFormat>,
) -> Vec<SubtitleTrack> {
    use std::collections::HashMap;
    let mut best: HashMap<String, SubtitleTrack> = HashMap::new();
    let pref_ext = preferred_format.map(|f| f.as_ext().to_string());

    for track in tracks {
        let key = track.language.to_ascii_lowercase();
        let entry = best.entry(key);
        match entry {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(track);
            }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                let existing = e.get();
                let new_is_better = if let Some(ref pref) = pref_ext {
                    track.ext.eq_ignore_ascii_case(pref) && !existing.ext.eq_ignore_ascii_case(pref)
                } else {
                    // Prefer manual over auto
                    !track.is_auto && existing.is_auto
                };
                if new_is_better {
                    e.insert(track);
                }
            }
        }
    }

    best.into_values().collect()
}
