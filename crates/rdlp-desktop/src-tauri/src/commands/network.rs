//! Network overrides shared by the IPC commands.
//!
//! The desktop's cookie source lives in [`AppSettings`], not in the config
//! file the shared `RdlpClient` was built from, so every command that needs it
//! must pass it per call. This module owns the one definition of "which
//! settings fields are the cookie source" so downloads and search cannot
//! drift apart — they did, and search silently ignored the setting (#691).

use rdlp_api::request::NetworkOptions;

use crate::state::AppSettings;

/// Build the cookie-carrying [`NetworkOptions`] for the persisted settings.
///
/// Only the cookie fields are set: every other field stays `None` so the
/// client's base config keeps whatever it already holds (the `Option`-as-
/// inherit overlay of #610).
pub(crate) fn cookies_network_options(settings: &AppSettings) -> NetworkOptions {
    NetworkOptions {
        cookies_from_browser: settings.cookies_from_browser,
        cookies_file: settings.cookies_file.clone(),
        ..NetworkOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rdlp_types::BrowserType;

    use super::cookies_network_options;
    use crate::state::AppSettings;

    /// The browser cookie source reaches the `NetworkOptions`.
    #[test]
    fn maps_cookies_from_browser() {
        let settings = AppSettings {
            cookies_from_browser: Some(BrowserType::Firefox),
            ..AppSettings::default()
        };
        let net = cookies_network_options(&settings);
        assert_eq!(net.cookies_from_browser, Some(BrowserType::Firefox));
    }

    /// The cookie-file source reaches the `NetworkOptions`.
    #[test]
    fn maps_cookies_file() {
        let settings = AppSettings {
            cookies_file: Some(PathBuf::from("/settings/cookies.txt")),
            ..AppSettings::default()
        };
        let net = cookies_network_options(&settings);
        assert_eq!(
            net.cookies_file,
            Some(PathBuf::from("/settings/cookies.txt"))
        );
    }

    /// Unset settings produce no cookie source — an empty override must not
    /// clobber a cookie source the base config already carries.
    #[test]
    fn unset_settings_produce_no_cookie_source() {
        let net = cookies_network_options(&AppSettings::default());
        assert_eq!(net.cookies_from_browser, None);
        assert_eq!(net.cookies_file, None);
    }

    /// No non-cookie field is set: this override is cookies-only, so a
    /// timeout or proxy in the base config survives it.
    #[test]
    fn sets_no_non_cookie_fields() {
        let settings = AppSettings {
            cookies_from_browser: Some(BrowserType::Chrome),
            proxy: Some("http://settings:3128".to_owned()),
            socket_timeout: Some(42),
            ..AppSettings::default()
        };
        let net = cookies_network_options(&settings);
        assert_eq!(net.proxy, None);
        assert_eq!(net.timeout_secs, None);
        assert_eq!(net.rate_limit, None);
        assert_eq!(net.retries, None);
    }
}
