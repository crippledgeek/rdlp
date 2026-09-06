//! Network overrides shared by the IPC commands.
//!
//! The desktop's network settings live in [`AppSettings`], not in the config
//! file the shared `RdlpClient` was built from, so every command that needs
//! them must pass them per call. This module owns the one definition of
//! "what the persisted settings mean as a `NetworkOptions`" so no command can
//! quietly honour a different subset than its neighbours — search honoured
//! none of them, which is how a set cookie source reached nothing (#691).

use rdlp_api::request::NetworkOptions;

use crate::error::AppError;
use crate::state::AppSettings;
use rdlp_types::boundary::Action;

/// Build the [`NetworkOptions`] the persisted settings imply, validating the
/// proxy at the IPC boundary.
///
/// Every field is `Option`: `None` means "preserve whatever the base `Config`
/// already holds", so an unset setting never clobbers a config-file value
/// (the `Option`-as-inherit overlay of #610).
///
/// `rate_limit` is parsed here from the settings' human string (`"500K"`).
/// A caller that also accepts a per-request rate limit must choose between
/// the two *strings* before parsing, so an unparseable request value does not
/// silently fall back to the settings value — see `build_network_options`.
///
/// # Errors
///
/// Returns [`AppError::InvalidInput`] if the settings carry an unusable proxy
/// URL. Returning the overlay and the validation together is deliberate: a
/// command cannot obtain one without the other. Only the download command
/// checked the proxy, so a malformed one failed loudly there and was silently
/// dropped by `rdlp-http` on every other path — the same "the setting does
/// nothing" surprise as #691.
pub(crate) fn settings_network_options(settings: &AppSettings) -> Result<NetworkOptions, AppError> {
    if let Some(proxy) = &settings.proxy {
        validate_proxy(proxy)?;
    }
    Ok(NetworkOptions {
        cookies_from_browser: settings.cookies_from_browser,
        cookies_file: settings.cookies_file.clone(),
        proxy: settings.proxy.clone(),
        rate_limit: settings
            .rate_limit
            .as_deref()
            .and_then(super::download::parse_rate_limit),
        timeout_secs: settings.socket_timeout,
        read_timeout_secs: settings.read_timeout,
        pool_idle_timeout_secs: settings.pool_idle_timeout,
        download_timeout_secs: settings.download_timeout,
        merge_timeout_secs: settings.merge_timeout,
        concurrent_fragments: settings.concurrent_fragments,
        buffer_size: settings.buffer_size,
        parallel_threshold: settings.parallel_threshold,
        hls_head_probe_timeout: settings.hls_head_probe_timeout,
        ..NetworkOptions::default()
    })
}

/// Validate one proxy URL at the IPC boundary.
///
/// `rdlp-http` re-checks every proxy before building its client and falls
/// back to no proxy on failure, so this is not the security boundary — it is
/// what turns that silent fallback into an error the operator can act on.
pub(crate) fn validate_proxy(proxy: &str) -> Result<(), AppError> {
    // `validate_proxy`, not `network_check`: the action names the operation
    // that failed, and nothing in the tree is called `network_check`. This
    // fires wherever settings are turned into `NetworkOptions` — analyze,
    // search, and download — so the action is the only thing in the record
    // that says which check refused.
    rdlp_security::validate_proxy_url(proxy)
        .map_err(|e| AppError::invalid_input(Action::new("validate_proxy"), "proxy", e))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rdlp_types::BrowserType;

    use super::settings_network_options;
    use crate::state::AppSettings;

    /// The browser cookie source reaches the `NetworkOptions`.
    #[test]
    fn maps_cookies_from_browser() {
        let settings = AppSettings {
            cookies_from_browser: Some(BrowserType::Firefox),
            ..AppSettings::default()
        };
        let net = settings_network_options(&settings).expect("valid settings");
        assert_eq!(net.cookies_from_browser, Some(BrowserType::Firefox));
    }

    /// The cookie-file source reaches the `NetworkOptions`.
    #[test]
    fn maps_cookies_file() {
        let settings = AppSettings {
            cookies_file: Some(PathBuf::from("/settings/cookies.txt")),
            ..AppSettings::default()
        };
        let net = settings_network_options(&settings).expect("valid settings");
        assert_eq!(
            net.cookies_file,
            Some(PathBuf::from("/settings/cookies.txt"))
        );
    }

    /// The proxy reaches the `NetworkOptions` — a GUI-configured proxy must
    /// apply to every command, not only to downloads.
    #[test]
    fn maps_proxy() {
        let settings = AppSettings {
            proxy: Some("http://proxy.example.com:3128".to_owned()),
            ..AppSettings::default()
        };
        let net = settings_network_options(&settings).expect("valid settings");
        assert_eq!(net.proxy, Some("http://proxy.example.com:3128".to_owned()));
    }

    /// The rate limit is parsed from the settings' human string.
    #[test]
    fn maps_and_parses_rate_limit() {
        let settings = AppSettings {
            rate_limit: Some("500K".to_owned()),
            ..AppSettings::default()
        };
        let net = settings_network_options(&settings).expect("valid settings");
        assert_eq!(net.rate_limit, Some(500 * 1024));
    }

    /// An unparseable rate-limit string yields no override rather than a
    /// bogus one — the base config's value survives.
    #[test]
    fn unparseable_rate_limit_yields_no_override() {
        let settings = AppSettings {
            rate_limit: Some("not-a-rate".to_owned()),
            ..AppSettings::default()
        };
        assert_eq!(
            settings_network_options(&settings)
                .expect("valid settings")
                .rate_limit,
            None
        );
    }

    /// The timeouts reach the `NetworkOptions`.
    #[test]
    fn maps_timeouts() {
        let settings = AppSettings {
            socket_timeout: Some(42),
            read_timeout: Some(43),
            ..AppSettings::default()
        };
        let net = settings_network_options(&settings).expect("valid settings");
        assert_eq!(net.timeout_secs, Some(42));
        assert_eq!(net.read_timeout_secs, Some(43));
    }

    /// Unset settings produce no override at all — an empty overlay must not
    /// clobber what the base config already carries (#610).
    #[test]
    fn unset_settings_produce_no_overrides() {
        let net = settings_network_options(&AppSettings::default()).expect("valid settings");
        assert_eq!(net.cookies_from_browser, None);
        assert_eq!(net.cookies_file, None);
        assert_eq!(net.proxy, None);
        assert_eq!(net.rate_limit, None);
        assert_eq!(net.timeout_secs, None);
        assert_eq!(net.read_timeout_secs, None);
        assert_eq!(net.pool_idle_timeout_secs, None);
        assert_eq!(net.download_timeout_secs, None);
        assert_eq!(net.merge_timeout_secs, None);
        assert_eq!(net.concurrent_fragments, None);
        assert_eq!(net.buffer_size, None);
        assert_eq!(net.parallel_threshold, None);
        assert_eq!(net.hls_head_probe_timeout, None);
        assert_eq!(net.retries, None);
    }

    /// A proxy pointing at a private host is rejected at the boundary rather
    /// than silently dropped further down.
    #[test]
    fn rejects_a_private_host_proxy() {
        let settings = AppSettings {
            proxy: Some("http://127.0.0.1:3128".to_owned()),
            ..AppSettings::default()
        };
        let err = settings_network_options(&settings).expect_err("private host must be rejected");
        assert!(matches!(err, crate::error::AppError::InvalidInput { .. }));
    }

    /// An unusable proxy scheme is rejected too.
    #[test]
    fn rejects_an_unsupported_proxy_scheme() {
        let settings = AppSettings {
            proxy: Some("ftp://proxy.example.com:3128".to_owned()),
            ..AppSettings::default()
        };
        assert!(settings_network_options(&settings).is_err());
    }

    /// `retries` has no settings field, so it is never set here: a base-config
    /// retry count must survive the overlay.
    #[test]
    fn never_sets_retries() {
        let settings = AppSettings {
            socket_timeout: Some(42),
            ..AppSettings::default()
        };
        assert_eq!(
            settings_network_options(&settings)
                .expect("valid settings")
                .retries,
            None
        );
    }
}
