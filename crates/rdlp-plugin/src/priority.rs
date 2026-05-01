//! Plugin priority validation + per-URL override clamping.
//!
//! See design spec §8.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(clippy::doc_markdown)]

use crate::manifest::Manifest;
use url::Url;

/// Maximum priority reserved for rdlp built-in extractors.
pub const BUILT_IN_MAX: u32 = 99;
/// Minimum priority a third-party plugin may declare in its manifest.
pub const PLUGIN_MIN: u32 = 100;
/// Maximum priority a third-party plugin may declare in its manifest.
pub const PLUGIN_MAX: u32 = 199;
/// Maximum priority a user override may set in `plugin-priorities.toml`.
pub const USER_MAX: u32 = 255;

/// Compute the effective dispatch priority of a plugin for a specific URL.
///
/// Inputs:
/// - `manifest` — the plugin's signed manifest (priority + claims_override)
/// - `url` — the URL being dispatched
/// - `built_in_claims_url` — does any built-in extractor's pattern cover this URL?
///   (resolved by the orchestrator before calling this function)
/// - `user_override` — optional value from `~/.config/rdlp/plugin-priorities.toml`,
///   capped to `USER_MAX`
///
/// Returns the priority to use when sorting candidate extractors. Higher = wins.
#[must_use]
pub fn effective_priority(
    manifest: &Manifest,
    url: &Url,
    built_in_claims_url: bool,
    user_override: Option<u32>,
) -> u32 {
    if let Some(user) = user_override {
        return user.min(USER_MAX);
    }
    let host = url.host_str().unwrap_or("");
    let plugin_overrides_this_host = manifest
        .claims_override
        .iter()
        .any(|h| host == h || host.ends_with(&format!(".{h}")));
    if built_in_claims_url && !plugin_overrides_this_host {
        manifest.priority.min(BUILT_IN_MAX)
    } else {
        manifest.priority
    }
}
