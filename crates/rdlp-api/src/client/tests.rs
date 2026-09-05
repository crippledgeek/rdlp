//! Tests for `RdlpClient` and `RdlpClientBuilder`

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
    let _clone = client;
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

    let config = client
        .build_config(&request)
        .expect("merged config must be valid");
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
    let config = client
        .build_config(&request)
        .expect("merged config must be valid");

    assert!(config.verbose);
    assert!(config.overwrite);
}

#[test]
fn test_build_config_preserves_remux_from_base_config() {
    use rdlp_types::ContainerFormat;

    let mut base_config = Config::default();
    base_config.postprocess.remux_container = Some(ContainerFormat::Mkv);
    base_config.postprocess.embed_metadata = true;
    base_config.postprocess.normalize_audio = true;

    let client = RdlpClient::builder().config(base_config).build().unwrap();
    // Default request has all None — should NOT overwrite config values
    let request = DownloadRequest::new("http://example.com");
    let config = client
        .build_config(&request)
        .expect("merged config must be valid");

    assert_eq!(
        config.postprocess.remux_container,
        Some(ContainerFormat::Mkv),
        "remux_container must be preserved from base config"
    );
    assert!(
        config.postprocess.embed_metadata,
        "embed_metadata must be preserved from base config"
    );
    assert!(
        config.postprocess.normalize_audio,
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
    let config = client
        .build_config(&request)
        .expect("merged config must be valid");
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
    let config = client
        .build_config(&request)
        .expect("merged config must be valid");
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
    let config = client
        .build_config(&request)
        .expect("merged config must be valid");
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

/// A request-level `cookies_from_browser` reaches the merged config.
///
/// What remains of a former test that also re-asserted
/// `config.cookies_from_browser.is_some() || config.cookies_file.is_some()`
/// against configs it had just built with those fields — a tautology over
/// the test's own values that no production change could fail. This half
/// exercises `build_config`, so it can.
#[test]
fn test_build_config_propagates_cookies_from_browser() {
    use rdlp_types::BrowserType;

    let client = RdlpClient::new(Config::default()).unwrap();
    let mut request = DownloadRequest::new("http://example.com");
    request.network.cookies_from_browser = Some(BrowserType::Firefox);
    let merged = client
        .build_config(&request)
        .expect("merged config must be valid");
    assert_eq!(
        merged.cookies_from_browser,
        Some(BrowserType::Firefox),
        "cookies_from_browser must propagate through build_config"
    );
}

/// A config with no cookie source configured must not fail on cookies.
///
/// `load_cookies` is a no-op when neither source is set, so the sanitizing
/// loader must return `Ok` and the download must reach extraction (where it
/// fails on the unsupported URL, which is expected). This is the guard that
/// the always-fatal cookie handling did not become fatal for downloads that
/// never asked for cookies.
#[tokio::test]
async fn test_download_without_cookie_source_does_not_fail_on_cookies() {
    let client = RdlpClient::new(Config::default()).unwrap();
    let request = DownloadRequest::new("http://example.com/video");
    let mut handle = client.download(request);

    let mut got_cookie_failed = false;
    while let Some(event) = handle.events().recv().await {
        if let Event::Failed {
            error: RdlpApiError::IoError { message },
            ..
        } = &event
            && message.contains("cookie")
        {
            got_cookie_failed = true;
        }
    }
    assert!(
        !got_cookie_failed,
        "must not get a cookie-related Failed event when no cookie source is configured"
    );
}
