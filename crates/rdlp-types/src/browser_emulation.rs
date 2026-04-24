//! Browser fingerprint selection for the rdlp HTTP stack.
//!
//! See docs/superpowers/specs/2026-04-24-tls-impersonation-design.md §6.5.

use serde::{Deserialize, Serialize};
use wreq_util::Emulation;

/// Browser fingerprint profile for the HTTP stack.
///
/// Selects which browser's TLS handshake, HTTP/2 frame shape, and header
/// order is emulated on every outgoing request. The `*Latest` variants
/// track the most recent stable profile the `wreq-util` crate ships;
/// `Pinned` locks to a specific historical profile for debugging /
/// regression analysis.
///
/// Default is `ChromeLatest` so installs track Chrome's release cadence
/// automatically.
///
/// See docs/superpowers/specs/2026-04-24-tls-impersonation-design.md §6.5.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserEmulation {
    /// Tracks the latest Chrome profile shipped by wreq-util.
    #[default]
    ChromeLatest,
    /// Tracks the latest Firefox profile shipped by wreq-util.
    FirefoxLatest,
    /// Tracks the latest Safari profile shipped by wreq-util.
    SafariLatest,
    /// Pin to a specific `wreq-util` profile identifier (e.g. "chrome-137").
    /// Invalid identifiers fall back to `ChromeLatest` at resolve time.
    Pinned(String),
}

impl BrowserEmulation {
    /// Resolve to the concrete `wreq_util::Emulation` value that `wreq::ClientBuilder::emulation`
    /// expects. Called at each `HttpClientFactory` build.
    pub fn resolve(&self) -> Emulation {
        match self {
            // Update these bindings when wreq-util publishes newer profiles.
            // Currently bound to the newest stable desktop variants in
            // wreq-util 3.0.0-rc.10 (Chrome 139, Firefox 139, Safari 26.2).
            BrowserEmulation::ChromeLatest => Emulation::Chrome139,
            BrowserEmulation::FirefoxLatest => Emulation::Firefox139,
            BrowserEmulation::SafariLatest => Emulation::Safari26_2,
            BrowserEmulation::Pinned(name) => {
                Self::resolve_pinned(name).unwrap_or(Emulation::Chrome139)
            }
        }
    }

    /// Map a pinned identifier string to a concrete Emulation.
    /// Extend this as wreq-util adds variants.
    fn resolve_pinned(name: &str) -> Option<Emulation> {
        match name {
            "chrome-137" => Some(Emulation::Chrome137),
            "chrome-138" => Some(Emulation::Chrome138),
            "chrome-139" => Some(Emulation::Chrome139),
            "firefox-133" => Some(Emulation::Firefox133),
            "firefox-135" => Some(Emulation::Firefox135),
            "firefox-136" => Some(Emulation::Firefox136),
            "firefox-139" => Some(Emulation::Firefox139),
            "safari-26" => Some(Emulation::Safari26),
            "safari-26.1" => Some(Emulation::Safari26_1),
            "safari-26.2" => Some(Emulation::Safari26_2),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_chrome_latest() {
        let e = BrowserEmulation::default();
        assert!(matches!(e, BrowserEmulation::ChromeLatest));
    }

    #[test]
    fn serde_chrome_latest_kebab_case() {
        let e = BrowserEmulation::ChromeLatest;
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, "\"chrome-latest\"");
    }

    #[test]
    fn serde_firefox_latest() {
        let e = BrowserEmulation::FirefoxLatest;
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, "\"firefox-latest\"");
    }

    #[test]
    fn serde_pinned_is_externally_tagged() {
        let e = BrowserEmulation::Pinned("chrome-137".to_string());
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, "{\"pinned\":\"chrome-137\"}");
    }

    #[test]
    fn serde_roundtrip() {
        for variant in [
            BrowserEmulation::ChromeLatest,
            BrowserEmulation::FirefoxLatest,
            BrowserEmulation::SafariLatest,
            BrowserEmulation::Pinned("chrome-137".to_string()),
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            let back: BrowserEmulation = serde_json::from_str(&s).unwrap();
            assert_eq!(format!("{variant:?}"), format!("{back:?}"));
        }
    }

    #[test]
    fn resolve_returns_concrete_emulation() {
        // Smoke: each variant resolves without panicking.
        let _ = BrowserEmulation::ChromeLatest.resolve();
        let _ = BrowserEmulation::FirefoxLatest.resolve();
        let _ = BrowserEmulation::SafariLatest.resolve();
    }

    #[test]
    fn pinned_valid_resolves() {
        assert_eq!(
            format!("{:?}", BrowserEmulation::Pinned("chrome-137".into()).resolve()),
            format!("{:?}", Emulation::Chrome137)
        );
    }

    #[test]
    fn pinned_invalid_falls_back() {
        // Unknown pin maps to Chrome139 (current *Latest target).
        assert_eq!(
            format!("{:?}", BrowserEmulation::Pinned("nonsense".into()).resolve()),
            format!("{:?}", Emulation::Chrome139)
        );
    }
}
