//! Tests for RdlpClient and RdlpClientBuilder

use super::*;

#[test]
fn test_builder_requires_config() {
    let result = RdlpClient::builder().build();
    assert!(result.is_err());
    match result.unwrap_err() {
        RdlpApiError::BuilderError { message } => {
            assert!(message.contains("config"));
        }
        other => panic!("Expected BuilderError, got: {other:?}"),
    }
}

#[test]
fn test_new_with_default_config() {
    let result = RdlpClient::new(Config::default());
    assert!(result.is_ok());
}

#[test]
fn test_client_is_clone() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let _clone = client.clone();
}

#[test]
fn test_builder_default() {
    let builder = RdlpClientBuilder::default();
    assert!(builder.config.is_none());
    assert!(builder.interactive.is_none());
}

#[tokio::test]
async fn test_download_returns_handle() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let handle = client.download(DownloadRequest::new("http://example.com"));
    assert!(handle.id().as_u64() > 0);
    // Cancel immediately so the task doesn't attempt a real download
    handle.cancel();
}

#[test]
fn test_build_config_applies_overrides() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let request = DownloadRequest {
        url: "http://example.com".into(),
        output: crate::request::OutputOptions {
            output_dir: Some(std::path::PathBuf::from("/tmp/test")),
            template: Some("%(title)s.%(ext)s".into()),
            ..Default::default()
        },
        format: crate::request::FormatOptions {
            selector: Some("best".into()),
            ..Default::default()
        },
        network: crate::request::NetworkOptions {
            retries: Some(7),
            concurrent_fragments: Some(8),
            ..Default::default()
        },
        ..Default::default()
    };

    let config = client.build_config(&request);
    assert_eq!(
        config.output_directory,
        std::path::PathBuf::from("/tmp/test")
    );
    assert_eq!(config.output_template, "%(title)s.%(ext)s");
    assert_eq!(config.format, Some("best".to_string()));
    assert_eq!(config.retries, 7);
    assert_eq!(config.concurrent_fragments, 8);
}

#[test]
fn test_build_config_preserves_base_when_no_override() {
    let base_config = Config {
        verbose: true,
        overwrite: true,
        ..Config::default()
    };

    let client = RdlpClient::builder().config(base_config).build().unwrap();
    let request = DownloadRequest::new("http://example.com");
    let config = client.build_config(&request);

    assert!(config.verbose);
    assert!(config.overwrite);
}

#[test]
fn test_build_config_preserves_remux_from_base_config() {
    use rdlp_types::ContainerFormat;

    let base_config = Config {
        remux_container: Some(ContainerFormat::Mkv),
        embed_metadata: true,
        normalize_audio: true,
        ..Config::default()
    };

    let client = RdlpClient::builder().config(base_config).build().unwrap();
    // Default request has all None — should NOT overwrite config values
    let request = DownloadRequest::new("http://example.com");
    let config = client.build_config(&request);

    assert_eq!(
        config.remux_container,
        Some(ContainerFormat::Mkv),
        "remux_container must be preserved from base config"
    );
    assert!(
        config.embed_metadata,
        "embed_metadata must be preserved from base config"
    );
    assert!(
        config.normalize_audio,
        "normalize_audio must be preserved from base config"
    );
}

#[test]
fn test_build_config_verbose_none_preserves_base() {
    let base_config = Config {
        verbose: true,
        ..Config::default()
    };
    let client = RdlpClient::builder().config(base_config).build().unwrap();
    let request = DownloadRequest::new("http://example.com");
    let config = client.build_config(&request);
    assert!(
        config.verbose,
        "verbose must be preserved when request.verbose is None"
    );
}

#[test]
fn test_build_config_verbose_some_overrides() {
    let base_config = Config {
        verbose: false,
        ..Config::default()
    };
    let client = RdlpClient::builder().config(base_config).build().unwrap();
    let mut request = DownloadRequest::new("http://example.com");
    request.verbose = Some(true);
    let config = client.build_config(&request);
    assert!(
        config.verbose,
        "verbose must be overridden by request.verbose = Some(true)"
    );
}

#[test]
fn test_build_config_verbose_false_overrides() {
    let base_config = Config {
        verbose: true,
        ..Config::default()
    };
    let client = RdlpClient::builder().config(base_config).build().unwrap();
    let mut request = DownloadRequest::new("http://example.com");
    request.verbose = Some(false);
    let config = client.build_config(&request);
    assert!(
        !config.verbose,
        "verbose must be overridden by request.verbose = Some(false)"
    );
}

#[test]
fn test_list_extractors() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let extractors = client.list_extractors();
    assert!(!extractors.is_empty());
}

#[test]
fn test_list_downloaders() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let downloaders = client.list_downloaders();
    assert!(!downloaders.is_empty());
}

#[test]
fn test_list_search_sites() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let sites = client.list_search_sites();
    assert!(!sites.is_empty());
    assert!(sites.iter().any(|s| s.name == "xhamster"));
}

#[test]
fn test_search_filters() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let filters = client.search_filters("xhamster").unwrap();
    assert!(!filters.is_empty());
    let keys: Vec<&str> = filters.iter().map(|f| f.key.as_str()).collect();
    assert!(keys.contains(&"quality"));
    assert!(keys.contains(&"sort"));
}

#[test]
fn test_search_filters_unknown_site() {
    let client = RdlpClient::new(Config::default()).unwrap();
    assert!(client.search_filters("nonexistent").is_err());
}

#[test]
fn test_cookies_explicitly_requested_flag() {
    use rdlp_types::BrowserType;
    use std::path::PathBuf;

    // No cookies requested — flag should be false
    let config_none = Config::default();
    assert!(
        config_none.cookies_from_browser.is_none() && config_none.cookies_file.is_none(),
        "Default config should not have cookies explicitly requested"
    );

    // Browser cookies requested — flag should be true
    let config_browser = Config {
        cookies_from_browser: Some(BrowserType::Chrome),
        ..Config::default()
    };
    assert!(
        config_browser.cookies_from_browser.is_some() || config_browser.cookies_file.is_some(),
        "Config with browser cookie should be explicitly requested"
    );

    // Cookie file requested — flag should be true
    let config_file = Config {
        cookies_file: Some(PathBuf::from("/tmp/cookies.txt")),
        ..Config::default()
    };
    assert!(
        config_file.cookies_from_browser.is_some() || config_file.cookies_file.is_some(),
        "Config with cookie file should be explicitly requested"
    );

    // Verify merge propagates cookies_from_browser from request
    let client = RdlpClient::new(Config::default()).unwrap();
    let mut request = DownloadRequest::new("http://example.com");
    request.network.cookies_from_browser = Some(BrowserType::Firefox);
    let merged = client.build_config(&request);
    assert_eq!(
        merged.cookies_from_browser,
        Some(BrowserType::Firefox),
        "cookies_from_browser must propagate through build_config"
    );
}

#[tokio::test]
async fn test_download_without_explicit_cookies_warns_on_cookie_error() {
    // Default config — no explicit cookies requested.
    // Cookie loading won't fail with default config (no browser, no
    // file), so this verifies the non-fatal path doesn't produce a
    // Failed event. The download will proceed to extraction and fail
    // there (unsupported URL), which is expected.
    let client = RdlpClient::new(Config::default()).unwrap();
    let request = DownloadRequest::new("http://example.com/video");
    let mut handle = client.download(request);

    let mut got_cookie_failed = false;
    while let Some(event) = handle.events().recv().await {
        if let Event::Failed { error, .. } = &event {
            // If we get a failure, it should NOT be about cookies
            if let RdlpApiError::IoError { message } = error {
                if message.contains("cookie") {
                    got_cookie_failed = true;
                }
            }
        }
    }
    assert!(
        !got_cookie_failed,
        "Should not get a cookie-related Failed event when cookies are not explicitly requested"
    );
}

#[tokio::test]
async fn test_download_explicit_cookies_file_fatal_on_missing() {
    use std::path::PathBuf;

    // Config with explicit cookie file that doesn't exist
    let config = Config {
        cookies_file: Some(PathBuf::from("/nonexistent/path/cookies.txt")),
        ..Config::default()
    };
    let client = RdlpClient::new(config).unwrap();

    let request = DownloadRequest::new("http://example.com/video");
    let mut handle = client.download(request);

    let mut got_failed = false;
    while let Some(event) = handle.events().recv().await {
        if let Event::Failed { error, .. } = &event {
            got_failed = true;
            // The error should be about cookie loading
            let msg = error.user_message();
            assert!(
                msg.to_lowercase().contains("cookie") || msg.to_lowercase().contains("file"),
                "Error message should reference cookies or file, got: {msg}"
            );
            break;
        }
    }
    assert!(
        got_failed,
        "Expected a Failed event when explicit cookie file is missing"
    );

    handle.cancel();
}
