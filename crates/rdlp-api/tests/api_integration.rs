//! Integration tests for the rdlp-api public surface.
//!
//! Covers download lifecycle events, cancellation flow, and EventBus
//! fan-out with multiple subscribers.

use rdlp_api::bus::EventBus;
use rdlp_api::events::Event;
use rdlp_api::request::DownloadRequest;
use rdlp_api::{RdlpApiError, RdlpClient};
use rdlp_core::Config;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_client() -> RdlpClient {
    let config = Config {
        progress: false,
        ..Default::default()
    };
    RdlpClient::new(config).expect("client should build")
}

/// Classify an event into a string tag for assertion ordering.
fn event_tag(event: &Event) -> &'static str {
    match event {
        Event::Started { .. } => "started",
        Event::MetadataReady { .. } => "metadata_ready",
        Event::FormatSelected { .. } => "format_selected",
        Event::Progress { .. } => "progress",
        Event::PostProcessing { .. } => "post_processing",
        Event::SubtitlesFound { .. } => "subtitles_found",
        Event::SubtitlesMissing { .. } => "subtitles_missing",
        Event::Warning { .. } => "warning",
        Event::Completed { .. } => "completed",
        Event::Failed { .. } => "failed",
        Event::Cancelled { .. } => "cancelled",
        Event::PlaylistDetected { .. } => "playlist_detected",
        Event::PlaylistItemStarted { .. } => "playlist_item_started",
        Event::Retrying { .. } => "retrying",
    }
}

// ---------------------------------------------------------------------------
// Download lifecycle: unsupported URL → Failed event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_download_emits_failed_for_unsupported_url() {
    let client = test_client();
    let request = DownloadRequest::new("https://totally-unsupported-site.example.org/v/123");
    let mut handle = client.download(request);

    let mut tags = Vec::new();
    let download_id = handle.id();

    while let Some(event) = handle.events().recv().await {
        assert_eq!(
            event.download_id(),
            download_id,
            "All events should carry the same download ID"
        );
        tags.push(event_tag(&event).to_string());
    }

    // For an unsupported URL, the extractor lookup fails before
    // the state machine starts — so we get Failed (no Started).
    assert!(!tags.is_empty(), "Should have received at least one event");
    assert!(
        tags.contains(&"failed".to_string()),
        "Should contain a Failed event, got: {tags:?}"
    );

    // wait() should return an error
    let result = handle.wait().await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Download lifecycle: all events carry the correct download ID
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_all_events_carry_download_id() {
    let client = test_client();
    let request = DownloadRequest::new("https://id-check.example.org/video");
    let mut handle = client.download(request);
    let expected_id = handle.id();

    let mut count = 0;
    while let Some(event) = handle.events().recv().await {
        assert_eq!(
            event.download_id(),
            expected_id,
            "Event {count} has wrong download ID"
        );
        count += 1;
    }

    assert!(count > 0, "Should have received at least one event");
    let _ = handle.wait().await;
}

// ---------------------------------------------------------------------------
// Cancellation: cancel() before extraction completes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cancel_before_result() {
    let client = test_client();
    let request = DownloadRequest::new("https://cancel-test.example.org/video");
    let mut handle = client.download(request);

    // Cancel immediately
    handle.cancel();

    // Drain remaining events
    let mut tags = Vec::new();
    while let Some(event) = handle.events().recv().await {
        tags.push(event_tag(&event).to_string());
    }

    // The result should be an error (cancelled or failed due to no extractor)
    let result = handle.wait().await;
    assert!(result.is_err(), "Cancelled download should return an error");

    // Either UserCancelled or extraction failure — both are acceptable
    // depending on race timing
    match &result {
        Err(RdlpApiError::UserCancelled) => {
            // Cancellation won the race — expected path
        }
        Err(_) => {
            // Extraction failure reached first — also acceptable
        }
        Ok(_) => panic!("Should not succeed after cancel"),
    }
}

// ---------------------------------------------------------------------------
// Cancel token: clone and external cancellation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cancel_token_external() {
    let client = test_client();
    let request = DownloadRequest::new("https://cancel-token-test.example.org/video");
    let mut handle = client.download(request);

    // Get external cancel token
    let token = handle.cancel_token();

    // Cancel via the cloned token (simulates Tauri DownloadManager use case)
    token.cancel();

    // Drain events
    while let Some(_event) = handle.events().recv().await {}

    // Should be an error
    let result = handle.wait().await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// EventBus: multi-subscriber fan-out
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_event_bus_lifecycle_multi_subscriber() {
    let bus = EventBus::new(64);
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    // Get a real DownloadId by starting (and immediately cancelling) a download
    let client = test_client();
    let handle = client.download(DownloadRequest::new("https://bus-test.example.org/v"));
    let id = handle.id();
    handle.cancel();

    // Simulate a full lifecycle event sequence
    let events = vec![
        Event::Started {
            id,
            url: "https://example.com/video".into(),
        },
        Event::FormatSelected {
            id,
            format_id: "hls-720".into(),
            quality: "720p".into(),
        },
        Event::Warning {
            id,
            message: "CDN redirect detected".into(),
        },
        Event::Cancelled { id },
    ];

    // Send all events
    for event in &events {
        let count = bus.send(event.clone());
        assert_eq!(count, 2, "Both subscribers should receive each event");
    }

    // Verify subscriber 1 receives all events in order
    for expected in &events {
        let received = rx1.recv().await.expect("rx1 should receive");
        assert_eq!(received.download_id(), expected.download_id());
        assert_eq!(event_tag(&received), event_tag(expected));
    }

    // Verify subscriber 2 receives all events in order
    for expected in &events {
        let received = rx2.recv().await.expect("rx2 should receive");
        assert_eq!(received.download_id(), expected.download_id());
        assert_eq!(event_tag(&received), event_tag(expected));
    }
}

#[tokio::test]
async fn test_event_bus_late_subscriber_misses_earlier_events() {
    let bus = EventBus::new(64);
    let mut rx1 = bus.subscribe();

    // Get a real DownloadId
    let client = test_client();
    let handle = client.download(DownloadRequest::new("https://late-sub.example.org/v"));
    let id = handle.id();
    handle.cancel();

    // Send first event (only rx1 is subscribed)
    let _ = bus.send(Event::Started {
        id,
        url: "https://example.com".into(),
    });

    // Now add second subscriber
    let mut rx2 = bus.subscribe();

    // Send second event (both subscribers)
    let _ = bus.send(Event::Cancelled { id });

    // rx1 should get both events
    let e1 = rx1.recv().await.unwrap();
    assert_eq!(event_tag(&e1), "started");
    let e2 = rx1.recv().await.unwrap();
    assert_eq!(event_tag(&e2), "cancelled");

    // rx2 should only get the second event
    let e = rx2.recv().await.unwrap();
    assert_eq!(event_tag(&e), "cancelled");
}

// ---------------------------------------------------------------------------
// Multiple concurrent downloads have distinct IDs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_concurrent_downloads_distinct_ids() {
    let client = test_client();

    let handle1 = client.download(DownloadRequest::new("https://a.example.org/1"));
    let handle2 = client.download(DownloadRequest::new("https://b.example.org/2"));

    assert_ne!(
        handle1.id(),
        handle2.id(),
        "Concurrent downloads must have distinct IDs"
    );

    // Clean up
    handle1.cancel();
    handle2.cancel();
}

// ---------------------------------------------------------------------------
// Client extract_info returns error for unsupported URL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_extract_info_unsupported_url() {
    let client = test_client();

    let result = client
        .extract_info("https://totally-unknown-site.example.com/video123")
        .await;
    assert!(result.is_err(), "Should fail for unsupported URL");

    match result.unwrap_err() {
        RdlpApiError::UnsupportedUrl { .. } | RdlpApiError::ExtractError { .. } => {
            // Expected
        }
        other => panic!("Expected UnsupportedUrl or ExtractError, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Client list_extractors and list_downloaders are non-empty
// ---------------------------------------------------------------------------

#[test]
fn test_client_lists_are_populated() {
    let client = test_client();
    assert!(!client.list_extractors().is_empty());
    assert!(!client.list_downloaders().is_empty());
}
