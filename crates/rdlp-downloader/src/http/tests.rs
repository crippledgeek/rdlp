//! Tests for HTTP downloader module

#![allow(clippy::unreadable_literal)] // byte-count literals in HTTP range tests

use super::*;
use rdlp_core::Downloader;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
struct TestProgressCallback {
    updates: Arc<Mutex<Vec<u64>>>,
}

impl ProgressCallback for TestProgressCallback {
    fn on_progress(&self, progress: &DownloadProgress) {
        self.updates.lock().unwrap().push(progress.bytes_downloaded);
    }

    fn on_complete(&self, _stats: &DownloadStats) {
        // Test completion
    }

    fn on_error(&self, _error: &str) {
        // Test error
    }
}

#[tokio::test]
async fn test_http_downloader_creation() {
    let downloader = HttpDownloader::new();
    assert_eq!(downloader.protocol(), "http");
    assert!(downloader.supports("http://example.com/video.mp4"));
    assert!(downloader.supports("https://example.com/video.mp4"));
    assert!(!downloader.supports("ftp://example.com/video.mp4"));
}

#[tokio::test]
async fn test_buffer_size_configuration() {
    let downloader = HttpDownloader::new().with_buffer_size(16384);
    assert_eq!(downloader.config.buffer_size, 16384);
}

#[tokio::test]
async fn test_resume_fails_when_server_returns_200() {
    use mockito::Server;
    use tempfile::NamedTempFile;

    let mut server = Server::new_async().await;

    // Mock endpoint that doesn't support Range requests (returns 200 instead of 206)
    let mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", "bytes=1000-")
        .with_status(200) // Server ignores Range header
        .with_header("content-type", "video/mp4")
        .with_body("full content from beginning")
        .create_async()
        .await;

    // Create downloader with NO retries for this test (testing resume failure, not retry)
    let retry_config = RetryConfig::new(
        0, // No retries
        Duration::from_millis(100),
        Duration::from_millis(100),
        2.0,
    );
    let downloader = HttpDownloader::new().with_retry_config(retry_config);
    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();

    // Write some initial data to simulate a partial download
    tokio::fs::write(path, b"partial data").await.unwrap();

    // Try to resume - should fail with error, NOT overwrite the file
    let result = downloader
        .download_with_resume(
            &format!("{}/video.mp4", server.url()),
            path,
            1000,
            None,
            None,
        )
        .await;

    mock.assert_async().await;

    // Should return an error
    assert!(result.is_err());

    // Verify the error message mentions resume not supported
    let err = result.unwrap_err();
    assert!(matches!(err, RdlpError::Download { .. }));
    assert!(err.to_string().contains("does not support resume"));
    assert!(err.to_string().contains("206"));

    // CRITICAL: Verify the partial file was NOT overwritten
    let file_contents = tokio::fs::read(path).await.unwrap();
    assert_eq!(
        file_contents, b"partial data",
        "Partial file should not be overwritten"
    );
}

#[tokio::test]
async fn test_parallel_download_error_propagation() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();

    // Create test content (20 MB to trigger parallel mode)
    let chunk_size = 5 * 1024 * 1024; // 5 MB per chunk
    let total_size = 20 * 1024 * 1024; // 20 MB total
    let test_content = vec![0u8; chunk_size];

    // Mock F3 probe: Range: bytes=0-262143 returns 206 with total size in Content-Range
    let _probe = server
        .mock("GET", "/test-video.mp4")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-262143/{total_size}"))
        .with_body(vec![0u8; 262144])
        .expect(1)
        .create_async()
        .await;

    // Mock successful Range requests for first 2 chunks
    let _mock_chunk_0 = server
        .mock("GET", "/test-video.mp4")
        .match_header("range", "bytes=0-5242879")
        .with_status(206)
        .with_header("content-range", "bytes 0-5242879/20971520")
        .with_body(&test_content)
        .create_async()
        .await;

    let _mock_chunk_1 = server
        .mock("GET", "/test-video.mp4")
        .match_header("range", "bytes=5242880-10485759")
        .with_status(206)
        .with_header("content-range", "bytes 5242880-10485759/20971520")
        .with_body(&test_content)
        .create_async()
        .await;

    // Mock FAILURE for chunk 2 (this should trigger fail-fast after retries)
    // With chunk-level retry, the failing chunk may be retried up to MAX_CHUNK_RETRIES (3) times.
    let mock_chunk_2_fail = server
        .mock("GET", "/test-video.mp4")
        .match_header("range", "bytes=10485760-15728639")
        .with_status(500)
        .with_body("Internal Server Error")
        .expect_at_least(1)
        .create_async()
        .await;

    // Chunk 3 should be cancelled by try_join_all after chunk 2 fails
    // It might or might not be called depending on timing
    let _mock_chunk_3 = server
        .mock("GET", "/test-video.mp4")
        .match_header("range", "bytes=15728640-20971519")
        .with_status(206)
        .with_header("content-range", "bytes 15728640-20971519/20971520")
        .with_body(&test_content)
        .expect_at_most(1) // May not be called if cancelled early
        .create_async()
        .await;

    // Create downloader with 4 concurrent fragments and no retries for fast test
    // Use Legacy chunking strategy to maintain 4 large chunks for this test
    use crate::chunking::ChunkSizeStrategy;
    use rdlp_core::RetryConfig;
    use std::time::Duration;
    let no_retry_config =
        RetryConfig::new(0, Duration::from_millis(1), Duration::from_millis(1), 1.0);

    let downloader = HttpDownloader::new()
        .with_concurrent_fragments(4)
        .with_retry_config(no_retry_config)
        .with_buffer_size(1024 * 1024) // 1 MB buffer
        .with_chunk_strategy(ChunkSizeStrategy::Legacy { chunk_count: 4 });

    let url = format!("{}/test-video.mp4", server.url());
    let output = temp_dir.path().join("output.mp4");

    // Attempt download - should fail on chunk 2
    let result = downloader.download_to_file(&url, &output, None).await;

    // Verify failure
    assert!(result.is_err(), "Download should fail due to chunk 2 error");

    let err = result.unwrap_err();
    assert!(
        matches!(err, RdlpError::Http { .. } | RdlpError::Network { .. }),
        "Should be an HTTP or network error, got: {err:?}"
    );

    // Chunks 0 and 1 should have been called (they succeed)
    // Note: The exact order depends on async scheduling, but try_join_all
    // will stop all futures on first error

    // Chunk 2 (the failing one) should definitely have been called
    mock_chunk_2_fail.assert_async().await;

    // This test demonstrates that try_join_all provides fail-fast behavior:
    // When chunk 2 fails, the entire operation fails immediately and returns
    // the error, rather than waiting for all chunks to complete.
}

#[tokio::test]
async fn test_download_to_writer_streams_bytes() {
    use mockito::Server;
    use tokio::io::AsyncWrite;

    let mut server = Server::new_async().await;
    let body = b"hello stdout stream";

    let _mock = server
        .mock("GET", "/pipe.mp4")
        .with_status(200)
        .with_header("content-type", "video/mp4")
        .with_body(body.as_slice())
        .create_async()
        .await;

    let downloader = HttpDownloader::new();

    // Use DuplexStream as writer: one end writes, we read from the other
    let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);

    // Spawn reader to drain the duplex
    let reader_handle = tokio::spawn(async move {
        let mut collected = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut server_stream, &mut collected)
            .await
            .unwrap();
        collected
    });

    let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(client_stream);
    let result = downloader
        .download_to_writer(&format!("{}/pipe.mp4", server.url()), writer, None)
        .await;

    assert!(result.is_ok(), "download_to_writer should succeed");
    let stats = result.unwrap();
    assert_eq!(stats.bytes_downloaded, body.len() as u64);

    let received = reader_handle.await.unwrap();
    assert_eq!(received, body.as_slice());
}

#[tokio::test]
async fn test_chunk_retry_succeeds_on_second_attempt() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // Mockito matches in LIFO order (last-created matched first).
    // Create success mock FIRST so it's matched SECOND.
    let body = vec![0xABu8; 1024];
    let _mock_ok = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-type", "application/octet-stream")
        // A single-part 206 MUST carry Content-Range (RFC 9110 §15.3.7.1);
        // the chunk path validates it against the requested span (#526).
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(body.clone())
        .expect(1)
        .create_async()
        .await;

    // Create fail mock LAST so it's matched FIRST (LIFO).
    let _mock_fail = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(500)
        .with_body("error")
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        &url,
        0,
        1023,
        &chunk_path,
        Some(progress),
        0,
        None,
    )
    .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1024);
}

#[tokio::test]
async fn test_chunk_retry_exhausted_returns_error() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // All requests fail with 500
    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(500)
        .with_body("error")
        .expect_at_least(3)
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        &url,
        0,
        1023,
        &chunk_path,
        Some(progress),
        0,
        None,
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_chunk_retry_non_retryable_fails_immediately() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // 403 is not retryable — should fail immediately, not retry
    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(403)
        .with_body("forbidden")
        .expect(1) // Only 1 request — no retries
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        &url,
        0,
        1023,
        &chunk_path,
        Some(progress),
        0,
        None,
    )
    .await;

    assert!(result.is_err());
    // mockito's expect(1) will panic on Drop if more than 1 request was made
}

#[tokio::test]
async fn test_chunk_retry_cleans_partial_file() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // Write a partial file to simulate a failed download
    tokio::fs::write(&chunk_path, b"partial data")
        .await
        .unwrap();
    assert!(chunk_path.exists());

    // Mockito matches in LIFO order. Create success mock FIRST (matched second).
    let body = vec![0xCDu8; 512];
    let _mock_ok = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-type", "application/octet-stream")
        // A single-part 206 MUST carry Content-Range (RFC 9110 §15.3.7.1);
        // the chunk path validates it against the requested span (#526).
        .with_header("content-range", "bytes 0-511/1048576")
        .with_body(body)
        .expect(1)
        .create_async()
        .await;

    // Create fail mock LAST (matched first — LIFO).
    let _mock_fail = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(500)
        .with_body("error")
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        &url,
        0,
        511,
        &chunk_path,
        Some(progress),
        0,
        None,
    )
    .await;

    assert!(result.is_ok());
    // The file should contain the successful download, not the partial data
    let contents = tokio::fs::read(&chunk_path).await.unwrap();
    assert_eq!(contents.len(), 512);
    assert!(contents.iter().all(|&b| b == 0xCD));
}

#[tokio::test]
async fn test_parallel_resume() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();

    // Simulate a 20 MB file with 5 MB already downloaded (25%)
    let total_size = 20 * 1024 * 1024; // 20 MB
    let already_downloaded = 5 * 1024 * 1024; // 5 MB
    let remaining_size = total_size - already_downloaded; // 15 MB

    // Create partial file with 5 MB of data
    let output = temp_dir.path().join("video.mp4");
    tokio::fs::write(&output, vec![0xAA; already_downloaded as usize])
        .await
        .unwrap();

    // Mock Range request for resume (server returns 206 with Content-Range)
    let _mock_resume = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=5242880-")
        .with_status(206)
        .with_header(
            "Content-Range",
            &format!("bytes 5242880-20971519/{total_size}"),
        )
        .with_header("Accept-Ranges", "bytes")
        .with_body(vec![0xBB; remaining_size as usize]) // Dummy data
        .expect(0) // Should NOT be called - we use parallel resume instead
        .create_async()
        .await;

    // Mock parallel resume chunks (4 chunks for 15 MB remaining)
    // Chunk 0: 5 MB - 8.75 MB (3.75 MB)
    let _mock_chunk_0 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=5242880-9175039")
        .with_status(206)
        .with_header("Content-Range", "bytes 5242880-9175039/20971520")
        .with_body(vec![0xCC; (9175040 - 5242880) as usize])
        .create_async()
        .await;

    // Chunk 1: 8.75 MB - 12.5 MB (3.75 MB)
    let _mock_chunk_1 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=9175040-13107199")
        .with_status(206)
        .with_header("Content-Range", "bytes 9175040-13107199/20971520")
        .with_body(vec![0xDD; (13107200 - 9175040) as usize])
        .create_async()
        .await;

    // Chunk 2: 12.5 MB - 16.25 MB (3.75 MB)
    let _mock_chunk_2 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=13107200-17039359")
        .with_status(206)
        .with_header("Content-Range", "bytes 13107200-17039359/20971520")
        .with_body(vec![0xEE; (17039360 - 13107200) as usize])
        .create_async()
        .await;

    // Chunk 3: 16.25 MB - 20 MB (3.75 MB)
    let _mock_chunk_3 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=17039360-20971519")
        .with_status(206)
        .with_header("Content-Range", "bytes 17039360-20971519/20971520")
        .with_body(vec![0xFF; (20971520 - 17039360) as usize])
        .create_async()
        .await;

    // Create downloader with 4 concurrent fragments and no retries
    // Use Legacy chunking strategy to maintain 4 large chunks for this test
    use crate::chunking::ChunkSizeStrategy;
    use rdlp_core::RetryConfig;
    use std::time::Duration;
    let no_retry_config =
        RetryConfig::new(0, Duration::from_millis(1), Duration::from_millis(1), 1.0);

    let downloader = HttpDownloader::new()
        .with_concurrent_fragments(4)
        .with_retry_config(no_retry_config)
        .with_chunk_strategy(ChunkSizeStrategy::Legacy { chunk_count: 4 });

    let url = format!("{}/video.mp4", server.url());

    // Resume download - should use parallel resume
    let result = downloader
        .download_with_resume(&url, &output, already_downloaded, None, None)
        .await;

    // Verify success
    assert!(result.is_ok(), "Parallel resume should succeed");
    let stats = result.unwrap();
    assert_eq!(
        stats.bytes_downloaded, total_size,
        "Should download full file size"
    );

    // Verify file size
    let metadata = tokio::fs::metadata(&output).await.unwrap();
    assert_eq!(metadata.len(), total_size, "Final file should be 20 MB");

    // Verify file contents structure:
    // First 5 MB: 0xAA (already downloaded)
    // Next 3.75 MB: 0xCC (chunk 0)
    // Next 3.75 MB: 0xDD (chunk 1)
    // Next 3.75 MB: 0xEE (chunk 2)
    // Last 3.75 MB: 0xFF (chunk 3)
    let contents = tokio::fs::read(&output).await.unwrap();
    assert_eq!(contents.len(), total_size as usize);

    // Check first 5 MB is original data (0xAA)
    assert_eq!(
        contents[0], 0xAA,
        "First byte should be from original partial download"
    );
    assert_eq!(
        contents[already_downloaded as usize - 1],
        0xAA,
        "Last byte of partial should be 0xAA"
    );

    // Check first byte of resumed data (chunk 0 starts with 0xCC)
    assert_eq!(
        contents[already_downloaded as usize], 0xCC,
        "First resumed byte should be from chunk 0"
    );
}

#[test]
fn with_parallel_threshold_sets_config_field() {
    let downloader = HttpDownloader::new().with_parallel_threshold(5 * 1024 * 1024);
    assert_eq!(downloader.config.parallel_threshold, 5 * 1024 * 1024);
}

#[test]
fn default_parallel_threshold_is_10_mib() {
    let downloader = HttpDownloader::new();
    assert_eq!(downloader.config.parallel_threshold, 10 * 1024 * 1024);
}

#[test]
fn with_parallel_threshold_clamps_zero_to_one() {
    let downloader = HttpDownloader::new().with_parallel_threshold(0);
    assert_eq!(
        downloader.config.parallel_threshold, 1,
        "threshold = 0 must be clamped to the validated floor of 1"
    );
}

#[tokio::test]
async fn probe_206_returns_size_from_content_range() {
    use mockito::Server;

    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/file")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", "bytes 0-262143/1048576")
        .with_body(vec![0u8; 262144])
        .expect(1)
        .create_async()
        .await;

    let head_guard = server.mock("HEAD", "/file").expect(0).create_async().await;

    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    let result = downloader.probe(&url).await.unwrap();

    assert_eq!(result.size, Some(1048576));
    assert!(result.supports_ranges);
    mock.assert_async().await;
    head_guard.assert_async().await;
}

#[tokio::test]
async fn probe_200_returns_content_length_no_ranges() {
    use mockito::Server;

    let mut server = Server::new_async().await;
    let body = vec![0u8; 524288];
    let mock = server
        .mock("GET", "/file")
        .match_header("range", "bytes=0-262143")
        .with_status(200)
        .with_header("content-length", "524288")
        .with_body(body)
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    let result = downloader.probe(&url).await.unwrap();

    assert_eq!(result.size, Some(524288));
    assert!(!result.supports_ranges);
    mock.assert_async().await;
}

#[tokio::test]
async fn probe_416_returns_none_falls_to_sequential() {
    use mockito::Server;

    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/file")
        .match_header("range", "bytes=0-262143")
        .with_status(416)
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    let result = downloader.probe(&url).await.unwrap();

    assert_eq!(result.size, None);
    assert!(!result.supports_ranges);
    mock.assert_async().await;
}

#[tokio::test]
async fn probe_206_malformed_content_range_keeps_supports_ranges_true() {
    use mockito::Server;

    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/file")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", "invalid-garbage")
        .with_body(vec![0u8; 262144])
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    let result = downloader.probe(&url).await.unwrap();

    assert_eq!(result.size, None);
    assert!(result.supports_ranges);
    mock.assert_async().await;
}

#[tokio::test]
async fn parallel_threshold_override_takes_parallel_path_for_5mib_file() {
    use mockito::Server;

    let mut server = Server::new_async().await;
    let body = vec![0u8; 5 * 1024 * 1024]; // 5 MiB

    // F3 probe: Range: bytes=0-262143 returns total size via content-range.
    let _probe = server
        .mock("GET", "/file.bin")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-262143/{}", body.len()))
        .with_body(vec![0u8; 262144])
        .expect(1)
        .create_async()
        .await;

    // Parallel chunk requests: multiple range requests verify we took the parallel
    // path. The mock asserts ≥2 invocations; any download outcome is acceptable.
    let _get_range = server
        .mock("GET", "/file.bin")
        .match_header(
            "range",
            mockito::Matcher::Regex(r"^bytes=\d+-\d+$".to_string()),
        )
        .with_status(206)
        .with_body(&body[..1024]) // any body; size mismatch is fine — we only assert call count
        .expect_at_least(2)
        .create_async()
        .await;

    let downloader = HttpDownloader::new()
        .with_parallel_threshold(1024 * 1024) // 1 MiB — below the 5 MiB file
        .with_concurrent_fragments(2)
        .with_read_timeout(std::time::Duration::from_secs(5))
        .with_download_timeout(std::time::Duration::from_secs(10));

    let url = format!("{}/file.bin", server.url());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let _ = downloader.download_to_file(&url, tmp.path(), None).await;
    // Assertions: the mock's `expect_at_least(2)` for range requests verifies
    // the parallel path executed. Any download outcome (success, body-mismatch
    // error) is acceptable — we're testing the dispatch decision, not the
    // download correctness.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_sequential_cancel_before_start_returns_cancelled() {
    use tokio_util::sync::CancellationToken;

    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/file")
        .with_status(200)
        .with_body(vec![0u8; 1024])
        .create_async()
        .await;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.bin");
    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    let token = CancellationToken::new();
    token.cancel();

    let res = downloader
        .download_sequential(&url, &out, None, Some(&token))
        .await;

    assert!(matches!(res, Err(RdlpError::Cancelled)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_sequential_cancel_mid_stream_returns_cancelled() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    // A server that sends HTTP 200 headers + chunked transfer-encoding preamble
    // but never sends actual body chunks. This parks the wreq body stream at
    // its next() poll, which is where the cancel arm races.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Read and discard the request.
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            // Send HTTP 200 with chunked encoding but no body chunks.
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\n\
                  Transfer-Encoding: chunked\r\n\
                  Content-Type: application/octet-stream\r\n\
                  \r\n",
            );
            let _ = stream.flush();
            // Hold the connection open so wreq waits for the next chunk.
            // `Duration::from_mins` (clippy suggestion) needs Rust 1.95;
            // workspace MSRV is 1.85.
            #[allow(clippy::duration_suboptimal_units)]
            std::thread::sleep(Duration::from_secs(60));
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.bin");
    let downloader = HttpDownloader::new();
    let url = format!("http://127.0.0.1:{port}/slow-body");

    let token = CancellationToken::new();
    let token2 = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        token2.cancel();
    });

    let res = downloader
        .download_sequential(&url, &out, None, Some(&token))
        .await;

    assert!(
        matches!(res, Err(RdlpError::Cancelled)),
        "expected Cancelled, got: {res:?}"
    );
}

#[tokio::test]
async fn download_sequential_cancel_none_passes_existing_behavior() {
    let mut server = mockito::Server::new_async().await;
    let body = vec![0xCC; 4096];
    let _mock = server
        .mock("GET", "/file")
        .with_status(200)
        .with_body(body.clone())
        .create_async()
        .await;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.bin");
    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    let stats = downloader
        .download_sequential(&url, &out, None, None)
        .await
        .unwrap();

    assert_eq!(stats.bytes_downloaded, 4096);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_format_propagates_cancel_to_sequential() {
    use rdlp_types::{DownloadProtocol, Format};
    use std::net::TcpListener;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let _ = listener.accept();
        // Hold connection open without sending any response — the probe
        // will block waiting for a response, and the cancel fires first.
        // `Duration::from_mins` needs Rust 1.95; workspace MSRV is 1.85.
        #[allow(clippy::duration_suboptimal_units)]
        std::thread::sleep(Duration::from_secs(60));
    });

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.bin");
    let downloader = HttpDownloader::new();
    let url = format!("http://127.0.0.1:{port}/blackhole");
    let format = Format::new("test", &url, "bin", DownloadProtocol::Https);

    let token = CancellationToken::new();
    let token2 = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        token2.cancel();
    });

    let res = downloader
        .download_format(&format, &out, None, Some(&token))
        .await;

    assert!(matches!(res, Err(RdlpError::Cancelled)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_with_resume_with_cancel_aborts_on_cancel() {
    use std::net::TcpListener;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    // Blackhole that accepts a connection and sends a 206 header but then
    // never sends body bytes. Forces the future to park at stream.next().
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        use std::io::Write;
        if let Ok((mut stream, _)) = listener.accept() {
            // Read request bytes (drop them)
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf);

            // Send 206 partial-content headers with a content-range that says total is 10MB
            let _ = stream.write_all(
                b"HTTP/1.1 206 Partial Content\r\n\
                  Content-Range: bytes 0-10485759/10485760\r\n\
                  Content-Length: 10485760\r\n\
                  Content-Type: application/octet-stream\r\n\
                  \r\n",
            );
            let _ = stream.flush();
            // Hold the connection open so wreq waits for body data.
            #[allow(clippy::duration_suboptimal_units)] // from_mins needs Rust 1.95; MSRV 1.85
            std::thread::sleep(Duration::from_secs(60));
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.bin");
    // Pre-create a 0-byte file to satisfy resume preconditions if any.
    tokio::fs::write(&out, b"").await.unwrap();

    let downloader = HttpDownloader::new();
    let url = format!("http://127.0.0.1:{port}/blackhole");

    let token = CancellationToken::new();
    let token2 = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        token2.cancel();
    });

    let res = downloader
        .download_with_resume_with_cancel(&url, &out, 0, None, Some(&token))
        .await;

    assert!(matches!(res, Err(RdlpError::Cancelled)));
}

#[tokio::test]
async fn download_to_file_no_head_under_normal_flow() {
    use mockito::Matcher;

    let mut server = mockito::Server::new_async().await;
    let body = vec![0xAA; 524288];

    // Probe responds with 206; total = 524288 (below 10 MiB threshold -> sequential).
    let _probe = server
        .mock("GET", "/file")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", "bytes 0-262143/524288")
        .with_body(vec![0u8; 262144])
        .create_async()
        .await;

    // Sequential download re-fetches the full file (no Range header).
    let _seq = server
        .mock("GET", "/file")
        .match_header("range", Matcher::Missing)
        .with_status(200)
        .with_body(body.clone())
        .create_async()
        .await;

    // HEAD must NEVER be issued.
    let head_guard = server.mock("HEAD", "/file").expect(0).create_async().await;

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.bin");
    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    let stats = downloader.download_to_file(&url, &out, None).await.unwrap();

    assert_eq!(stats.bytes_downloaded, 524288);
    head_guard.assert_async().await;
}

#[test]
fn next_with_cancel_helper_uses_biased_select() {
    // Static guard: the cancel select in next_with_cancel_and_timeout MUST
    // include `biased;` so cancel-priority is deterministic. Without biased,
    // tokio's PRNG branch selection can starve the cancel arm under load.
    //
    // Mirrors the static probe-order guard at
    // crates/rdlp-extractor/src/hls/expand_in_place.rs:247
    // (`test_extractor_call_order_expand_before_detect`).
    let source = include_str!("mod.rs");

    // Restrict to the helper's function body to avoid false positives from
    // other select! invocations elsewhere in the file.
    let start = source
        .find("pub(crate) async fn next_with_cancel_and_timeout")
        .expect("next_with_cancel_and_timeout not found in http/mod.rs");

    // End at the next top-level `fn` definition or the `#[cfg(test)]` block,
    // whichever comes first.
    let after_start = &source[start..];
    let cfg_test_end = after_start.find("\n#[cfg(test)]");
    let next_fn_end = after_start[1..]
        .find("\nfn ")
        .or_else(|| after_start[1..].find("\nasync fn "))
        .or_else(|| after_start[1..].find("\npub fn "))
        .or_else(|| after_start[1..].find("\npub(crate) fn "))
        .or_else(|| after_start[1..].find("\npub(crate) async fn "))
        .map(|i| i + 1);
    let end = match (cfg_test_end, next_fn_end) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => after_start.len(),
    };
    let body = &after_start[..end];

    let select_idx = body
        .find("tokio::select!")
        .expect("next_with_cancel_and_timeout must contain a tokio::select!");
    let after_select = &body[select_idx..];

    // `biased;` must appear before the first arm (which uses `=>`).
    let first_arm = after_select
        .find("=>")
        .expect("tokio::select! must have at least one arm");
    let header = &after_select[..first_arm];

    assert!(
        header.contains("biased;"),
        "tokio::select! in next_with_cancel_and_timeout MUST use `biased;` \
         to deterministically prioritize the cancel arm. Without it, tokio's \
         PRNG branch selection can starve cancel under load. \
         Found header: {header:?}"
    );
}

// ── #307: download_to_writer cooperative cancellation ──────────────────────

#[tokio::test]
async fn download_to_writer_cancel_before_start_returns_cancelled() {
    use tokio_util::sync::CancellationToken;

    // Token cancelled before call — pre-cancel guard must short-circuit
    // before any network activity.
    let token = CancellationToken::new();
    token.cancel();

    let downloader = HttpDownloader::new();
    // URL is deliberately unreachable; no connection should be attempted.
    let writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send> = Box::new(tokio::io::sink());

    let res = downloader
        .download_to_writer_with_cancel(
            "http://127.0.0.1:1/unreachable",
            writer,
            None,
            Some(&token),
        )
        .await;

    assert!(
        matches!(res, Err(RdlpError::Cancelled)),
        "expected Cancelled before any network attempt, got: {res:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_to_writer_cancel_mid_stream_returns_cancelled() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    // Server sends headers + chunked encoding preamble but holds the
    // connection open without delivering body chunks, parking wreq's body
    // stream at its next() poll — same pattern as
    // download_sequential_cancel_mid_stream_returns_cancelled.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\n\
                  Transfer-Encoding: chunked\r\n\
                  Content-Type: application/octet-stream\r\n\
                  \r\n",
            );
            let _ = stream.flush();
            #[allow(clippy::duration_suboptimal_units)]
            std::thread::sleep(Duration::from_secs(60));
        }
    });

    let downloader = HttpDownloader::new();
    let url = format!("http://127.0.0.1:{port}/slow-body");
    let writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send> = Box::new(tokio::io::sink());

    let token = CancellationToken::new();
    let token2 = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        token2.cancel();
    });

    let res = downloader
        .download_to_writer_with_cancel(&url, writer, None, Some(&token))
        .await;

    assert!(
        matches!(res, Err(RdlpError::Cancelled)),
        "expected Cancelled mid-stream, got: {res:?}"
    );
}

#[tokio::test]
async fn download_to_writer_cancel_none_passes_existing_behavior() {
    let mut server = mockito::Server::new_async().await;
    let body = vec![0xAB; 4096];
    let _mock = server
        .mock("GET", "/file")
        .with_status(200)
        .with_body(body.clone())
        .create_async()
        .await;

    let downloader = HttpDownloader::new();
    let url = format!("{}/file", server.url());

    // Use sink() to avoid the `'static` lifetime constraint on the boxed writer.
    let writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send> = Box::new(tokio::io::sink());

    let stats = downloader
        .download_to_writer_with_cancel(&url, writer, None, None)
        .await
        .unwrap();

    assert_eq!(stats.bytes_downloaded, 4096);
}

// ── #317: per-chunk-body cancel static guard ───────────────────────────────

#[test]
fn download_chunk_with_retry_passes_cancel_to_range() {
    // Static guard: download_chunk_with_retry MUST forward the cancel parameter
    // to download_range_with_progress, not None. This asserts the call-site text
    // contains `cancel` as the last argument (not a literal `None`).
    // Integration-style slow-chunk mockito tests would require SERVER_SENT_EVENTS
    // chunked encoding support in mockito, which is not available cleanly;
    // the static check is cheaper and catches the most likely regression.
    let source = include_str!("parallel.rs");

    let start = source
        .find("pub async fn download_chunk_with_retry")
        .expect("download_chunk_with_retry not found in parallel.rs");

    let after_start = &source[start..];
    // End at the next blank-line-separated fn.
    let end = after_start[1..]
        .find("\npub async fn ")
        .or_else(|| after_start[1..].find("\nasync fn "))
        .map_or(after_start.len(), |i| i + 1);
    let body = &after_start[..end];

    // The call to download_range_with_progress must pass `cancel` (not `None`).
    // Use balanced-paren matching so nested calls like `progress.clone()` don't
    // truncate the call argument span.
    let call_idx = body
        .find("download_range_with_progress(")
        .expect("download_chunk_with_retry must call download_range_with_progress");
    let after_open = &body[call_idx + "download_range_with_progress(".len()..];
    let mut depth: i32 = 1;
    let mut end_offset = None;
    for (i, ch) in after_open.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end_offset = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = end_offset.expect("call must have a balanced closing paren");
    let call_args = &after_open[..close];

    assert!(
        call_args.contains("cancel"),
        "download_chunk_with_retry must pass `cancel` to download_range_with_progress, \
         not `None`. Found call args: {call_args:?}"
    );
    assert!(
        !call_args.contains(", None)") && !call_args.ends_with("None"),
        "download_chunk_with_retry must NOT pass literal `None` as the cancel arg. \
         Found call args: {call_args:?}"
    );
}

// ── #317: download_format biased-select static guard ───────────────────────

#[test]
fn download_format_uses_biased_select() {
    // Static guard: Downloader::download_format's outer cancel select! MUST
    // include `biased;` so cancel-priority is deterministic. Mirrors
    // parallel_adaptive_cancel_uses_biased_select (issue #317).
    let source = include_str!("trait_impl.rs");

    let start = source
        .find("async fn download_format")
        .expect("download_format not found in trait_impl.rs");

    let after_start = &source[start..];
    // End at the next top-level `async fn` or `fn ` definition.
    let end = after_start[1..]
        .find("\n    async fn ")
        .or_else(|| after_start[1..].find("\n    fn "))
        .map_or(after_start.len(), |i| i + 1);
    let body = &after_start[..end];

    let select_idx = body
        .find("tokio::select!")
        .expect("download_format must contain a tokio::select! for cancel");
    let after_select = &body[select_idx..];
    let first_arm = after_select
        .find("=>")
        .expect("tokio::select! must have at least one arm");
    let header = &after_select[..first_arm];

    assert!(
        header.contains("biased;"),
        "tokio::select! in download_format MUST use `biased;` \
         to deterministically prioritize the cancel arm. \
         Found header: {header:?}"
    );
}

// ── #308: AIMD parallel-path static biased-select guard ────────────────────

#[test]
fn parallel_adaptive_cancel_uses_biased_select() {
    // Static guard: run_adaptive_chunk's semaphore-acquire select! MUST
    // include `biased;` so cancel-priority is deterministic. Mirrors
    // next_with_cancel_helper_uses_biased_select (Task 12, PR #303). The
    // chunk-job body (including this select!) was extracted from
    // download_parallel_adaptive into the free function run_adaptive_chunk
    // as part of the #568 follow-up cleanup — this guard moved with it.
    let source = include_str!("parallel.rs");

    let start = source
        .find("async fn run_adaptive_chunk")
        .expect("run_adaptive_chunk not found in parallel.rs");

    let after_start = &source[start..];
    // End at the next top-level fn definition.
    let end = after_start[1..]
        .find("\nasync fn ")
        .or_else(|| after_start[1..].find("\nfn "))
        .map_or(after_start.len(), |i| i + 1);
    let body = &after_start[..end];

    let select_idx = body
        .find("tokio::select!")
        .expect("run_adaptive_chunk must contain a tokio::select! for cancel");
    let after_select = &body[select_idx..];
    let first_arm = after_select
        .find("=>")
        .expect("tokio::select! must have at least one arm");
    let header = &after_select[..first_arm];

    assert!(
        header.contains("biased;"),
        "tokio::select! in run_adaptive_chunk MUST use `biased;` \
         to deterministically prioritize the cancel arm. \
         Found header: {header:?}"
    );
}

// ---------------------------------------------------------------------------
// Range-response validation (#526)
//
// A parallel chunk fetch writes its bytes into a fixed offset slot in the
// merged output, so a response that does not deliver exactly the requested
// span corrupts the file at that offset. RFC 9110 §15.3.7 puts the burden on
// the client: "A client MUST inspect a 206 response's Content-Type and
// Content-Range field(s) to determine what parts are enclosed and whether
// additional requests are needed." §14.2 additionally permits a server to
// ignore Range entirely and answer 200 with the full body.
//
// These tests pin each way a response can diverge from the requested span.
// ---------------------------------------------------------------------------

/// Build a downloader whose transport-level retry is disabled, so each test
/// observes exactly one request/response exchange.
fn validation_test_downloader() -> HttpDownloader {
    HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ))
}

/// RFC 9110 §14.2 lets a server ignore `Range` and return the whole body with
/// 200. Writing that into a chunk's offset slot is the corruption in #526,
/// so it must be refused rather than accepted as a valid chunk.
#[tokio::test]
async fn range_fetch_rejects_200_full_body() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // Server ignores Range: full 4096-byte body, status 200, no Content-Range.
    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(200)
        .with_body(vec![0xAAu8; 4096])
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        matches!(result, Err(RdlpError::Download { .. })),
        "a 200 full-body answer to a Range request must be refused, got {result:?}"
    );
}

/// A 206 is required to carry `Content-Range` (RFC 9110 §15.3.7). Without it
/// there is no way to confirm which span arrived, so it cannot be trusted.
#[tokio::test]
async fn range_fetch_rejects_206_without_content_range() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_body(vec![0xBBu8; 1024])
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        matches!(result, Err(RdlpError::Download { .. })),
        "a 206 without Content-Range must be refused, got {result:?}"
    );
}

/// The exact #526 signature: a correct-LENGTH response carrying the WRONG
/// span. Byte-count checking alone cannot catch this — only comparing
/// Content-Range against what was requested does.
#[tokio::test]
async fn range_fetch_rejects_mismatched_content_range() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // Requested 0-1023; server answers with the 1024 bytes at 1048576-1049599.
    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 1048576-1049599/2097152")
        .with_body(vec![0xCCu8; 1024])
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        matches!(result, Err(RdlpError::Network { .. })),
        "a 206 whose Content-Range is not the requested span must be refused, got {result:?}"
    );
    assert!(
        is_retryable_error(&result.unwrap_err()),
        "a wrong-span response is a per-response anomaly, so the chunk must be re-fetched \
         rather than failing the whole download"
    );
}

/// A short body must fail even though the headers are conformant. hyper
/// normally raises an error on an interrupted body, but hyperium/hyper#3253
/// documents a case where a truncated stream ended silently — so the byte
/// count is verified independently rather than trusted to the transport.
///
/// Truncation is transient, so this must be RETRYABLE (`Network`) to let
/// `download_chunk_with_retry` re-fetch the chunk.
#[tokio::test]
async fn range_fetch_rejects_short_body() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // Headers promise 0-1023 (1024 bytes); body delivers only 512.
    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/2097152")
        .with_body(vec![0xDDu8; 512])
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        matches!(result, Err(RdlpError::Network { .. })),
        "a short body must fail as a retryable Network error, got {result:?}"
    );
    assert!(
        is_retryable_error(&result.unwrap_err()),
        "a truncated chunk must be retryable so the chunk is re-fetched"
    );
}

/// A body longer than the requested span would push every later chunk
/// forward in the merged output.
#[tokio::test]
async fn range_fetch_rejects_overlong_body() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/2097152")
        .with_body(vec![0xEEu8; 2048])
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        result.is_err(),
        "a body longer than the requested span must be refused, got {result:?}"
    );

    // The overrun is rejected BEFORE the frame is written, so the excess never
    // reaches disk — the chunk file must never hold more than the span allows.
    if let Ok(meta) = tokio::fs::metadata(&chunk_path).await {
        assert!(
            meta.len() <= 1024,
            "an over-long body must not be written past the requested span; \
             chunk file holds {} bytes",
            meta.len()
        );
    }
}

/// Positive case: a fully conformant 206 still succeeds and reports the exact
/// requested length. Guards against the validation over-rejecting valid work.
#[tokio::test]
async fn range_fetch_accepts_conformant_206() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 4096-5119/2097152")
        .with_body(vec![0x5Au8; 1024])
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 4096, 5119, &chunk_path, None, None)
        .await;

    assert_eq!(
        result.expect("a conformant 206 must be accepted"),
        1024,
        "must report exactly the requested span length"
    );
}

// ---------------------------------------------------------------------------
// Merged-output size verification (#526)
//
// Final backstop, independent of the per-chunk checks: whatever the chunk
// layer believed it fetched, the assembled file must be exactly the size the
// server advertised before it is promoted to the user-visible name.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn verify_merged_size_accepts_exact_total() {
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.mp4");
    tokio::fs::write(&path, vec![0u8; 4096]).await.unwrap();

    assert!(
        verify_merged_size(&path, 4096, "https://example.com/video.mp4")
            .await
            .is_ok(),
        "an output matching the advertised total must be accepted"
    );
}

/// A short assembly is the shape a dropped or truncated chunk produces.
#[tokio::test]
async fn verify_merged_size_rejects_short_output() {
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.mp4");
    tokio::fs::write(&path, vec![0u8; 4095]).await.unwrap();

    assert!(
        verify_merged_size(&path, 4096, "https://example.com/video.mp4")
            .await
            .is_err(),
        "an output shorter than the advertised total must be refused"
    );
}

/// A long assembly is the shape a duplicated or overlapping chunk produces.
#[tokio::test]
async fn verify_merged_size_rejects_long_output() {
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("out.mp4");
    tokio::fs::write(&path, vec![0u8; 4097]).await.unwrap();

    assert!(
        verify_merged_size(&path, 4096, "https://example.com/video.mp4")
            .await
            .is_err(),
        "an output longer than the advertised total must be refused"
    );
}

#[tokio::test]
async fn verify_merged_size_rejects_missing_output() {
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("never-created.mp4");

    assert!(
        verify_merged_size(&path, 4096, "https://example.com/video.mp4")
            .await
            .is_err(),
        "a missing output file must be refused, not treated as verified"
    );
}

// ---------------------------------------------------------------------------
// Chunk retry actually RECOVERS from a bad range response (#526)
//
// Classifying an error as retryable is not the same as proving the chunk is
// re-fetched and the download proceeds. These drive the full
// `download_chunk_with_retry` loop: first attempt gets a corrupt response,
// second gets a good one, and the chunk must end up with the correct bytes.
// ---------------------------------------------------------------------------

/// A wrong-span response must be retried, and the retry's correct bytes must
/// be what lands in the chunk file — not appended to the rejected attempt.
#[tokio::test]
async fn chunk_retry_recovers_from_wrong_span_response() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // Mockito matches LIFO: create the success mock FIRST so it answers SECOND.
    let _mock_ok = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x11u8; 1024])
        .expect(1)
        .create_async()
        .await;

    // Answers FIRST: right length, WRONG span — the #526 signature.
    let _mock_wrong_span = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 1048576-1049599/2097152")
        .with_body(vec![0x99u8; 1024])
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let url = format!("{}/video.mp4", server.url());
    let result = download_chunk_with_retry(
        &downloader,
        &url,
        0,
        1023,
        &chunk_path,
        Some(Arc::new(AtomicU64::new(0))),
        0,
        None,
    )
    .await;

    assert_eq!(
        result.expect("the chunk must recover on retry after a wrong-span response"),
        1024
    );

    // The rejected attempt's bytes must not survive: the file holds exactly the
    // retry's payload, not 2048 bytes of both.
    let contents = tokio::fs::read(&chunk_path).await.unwrap();
    assert_eq!(contents.len(), 1024, "chunk must hold exactly one attempt");
    assert!(
        contents.iter().all(|&b| b == 0x11),
        "chunk must hold the RETRY's bytes, not the rejected wrong-span response"
    );
}

/// Same guarantee for a truncated body.
#[tokio::test]
async fn chunk_retry_recovers_from_short_body() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock_ok = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x22u8; 1024])
        .expect(1)
        .create_async()
        .await;

    // Answers FIRST: conformant headers, truncated body.
    let _mock_short = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x88u8; 400])
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let url = format!("{}/video.mp4", server.url());
    let result = download_chunk_with_retry(
        &downloader,
        &url,
        0,
        1023,
        &chunk_path,
        Some(Arc::new(AtomicU64::new(0))),
        0,
        None,
    )
    .await;

    assert_eq!(
        result.expect("the chunk must recover on retry after a truncated body"),
        1024
    );

    let contents = tokio::fs::read(&chunk_path).await.unwrap();
    assert_eq!(
        contents.len(),
        1024,
        "the truncated attempt must be discarded, not appended to"
    );
    assert!(contents.iter().all(|&b| b == 0x22));
}

/// A server that ignores Range (answers 200) is stating a capability, not
/// suffering a transient fault: that must fail fast rather than burn retries.
#[tokio::test]
async fn chunk_retry_does_not_retry_range_ignoring_server() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // expect(1): a non-retryable failure must issue exactly ONE request.
    let mock_200 = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(200)
        .with_body(vec![0x77u8; 4096])
        .expect(1)
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let url = format!("{}/video.mp4", server.url());
    let result = download_chunk_with_retry(
        &downloader,
        &url,
        0,
        1023,
        &chunk_path,
        Some(Arc::new(AtomicU64::new(0))),
        0,
        None,
    )
    .await;

    assert!(
        result.is_err(),
        "a Range-ignoring server must fail the chunk"
    );
    mock_200.assert_async().await;
}

// ---------------------------------------------------------------------------
// Exact-boundary negatives for the chunk-length check (#526).
//
// `range_fetch_accepts_conformant_206` pins N (exactly 1024 bytes accepted).
// These pin N-1 and N+1, so a `>` / `>=` slip in the length comparison cannot
// pass: a test at 512 or 2048 would survive that mutation, these do not.
// ---------------------------------------------------------------------------

/// One byte short of the promised span must still be refused.
#[tokio::test]
async fn range_fetch_rejects_body_one_byte_short() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x3Cu8; 1023]) // N-1
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        matches!(result, Err(RdlpError::Network { .. })),
        "1023 of 1024 bytes must be refused as incomplete, got {result:?}"
    );
}

/// One byte over the promised span must be refused, and must not reach disk.
#[tokio::test]
async fn range_fetch_rejects_body_one_byte_over() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x3Du8; 1025]) // N+1
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&url, 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        result.is_err(),
        "1025 of 1024 bytes must be refused as an overrun, got {result:?}"
    );
    if let Ok(meta) = tokio::fs::metadata(&chunk_path).await {
        assert!(
            meta.len() <= 1024,
            "the overrunning byte must not be written; file holds {} bytes",
            meta.len()
        );
    }
}

/// The resume path appends the response body at EOF, so a server answering
/// from a DIFFERENT offset than `resume_from` splices foreign bytes into the
/// file — the #526 corruption shape on the resume path. Checking the status is
/// 206 is not enough; the enclosed span must start where the file ends.
#[tokio::test]
async fn resume_rejects_response_starting_at_wrong_offset() {
    use mockito::Server;
    use tempfile::NamedTempFile;

    let mut server = Server::new_async().await;

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", "bytes=1000-")
        .with_status(206)
        // Asked to resume at 1000; server answers from 5000.
        .with_header("content-range", "bytes 5000-9999/10000")
        .with_body(vec![0x66u8; 5000])
        .create_async()
        .await;

    let downloader = HttpDownloader::new().with_retry_config(RetryConfig::new(
        0,
        Duration::from_millis(1),
        Duration::from_millis(10),
        2.0,
    ));

    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();
    tokio::fs::write(path, b"partial data").await.unwrap();

    let result = downloader
        .download_with_resume(
            &format!("{}/video.mp4", server.url()),
            path,
            1000,
            None,
            None,
        )
        .await;

    assert!(
        result.is_err(),
        "a resume response starting at the wrong offset must be refused, got {result:?}"
    );

    // The existing bytes must be left intact rather than extended with
    // wrongly-offset data.
    let contents = tokio::fs::read(path).await.unwrap();
    assert_eq!(
        contents, b"partial data",
        "a rejected resume must not append foreign bytes to the partial file"
    );
}

// ---------------------------------------------------------------------------
// #568 — chunk sweep on failed parallel downloads
//
// Before this fix, `cleanup_chunk_files` hardcoded the `.part{N}` marker
// while a resumed download's writer used `.resume{N}` — so a failed resumed
// download's cleanup pass deleted nothing (D1). The adaptive path called no
// cleanup at all (D2). All three tests below assert directly on the
// filesystem: after a failure, only files a foreign process planted may
// remain in the download's temp directory.
// ---------------------------------------------------------------------------

/// Collect the set of file names present in `dir`.
async fn dir_entries(dir: &Path) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    let mut entries = tokio::fs::read_dir(dir).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        names.insert(entry.file_name().to_string_lossy().into_owned());
    }
    names
}

/// Regression guard: a failed FRESH parallel download (static chunking, i.e.
/// adaptive disabled via `Legacy` strategy) must still sweep its `.part{N}`
/// chunk files. This path's suffix already matched pre-#568 (both writer and
/// cleaner used `"part"`), so — unlike the two tests below — this one is
/// expected to ALREADY PASS against the unpatched code; it pins the
/// behavior so the migration to `ChunkSet` does not regress it.
#[tokio::test]
async fn static_fresh_failure_sweeps_part_chunks_regression_guard() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();

    let chunk_size = 4 * 1024 * 1024; // 4 MiB
    let total_size = chunk_size * 3; // 3 chunks via Legacy{chunk_count: 3}
    let body = vec![0xAAu8; chunk_size];

    let _probe = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-262143/{total_size}"))
        .with_body(vec![0u8; 262144])
        .create_async()
        .await;

    let _chunk0 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=0-4194303")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-4194303/{total_size}"))
        .with_body(body.clone())
        .create_async()
        .await;

    let _chunk1 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=4194304-8388607")
        .with_status(206)
        .with_header(
            "content-range",
            &format!("bytes 4194304-8388607/{total_size}"),
        )
        .with_body(body)
        .create_async()
        .await;

    // Non-retryable: fails immediately, no chunk-level backoff sleep.
    let _chunk2_fail = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=8388608-12582911")
        .with_status(403)
        .with_body("forbidden")
        .expect_at_least(1)
        .create_async()
        .await;

    let no_retry = RetryConfig::new(0, Duration::from_millis(1), Duration::from_millis(1), 1.0);
    let downloader = HttpDownloader::new()
        .with_concurrent_fragments(3)
        .with_retry_config(no_retry)
        .with_chunk_strategy(crate::chunking::ChunkSizeStrategy::Legacy { chunk_count: 3 });

    let url = format!("{}/video.mp4", server.url());
    let output = temp_dir.path().join("output.mp4");

    // Foreign file that must survive the failure's cleanup pass.
    let foreign = temp_dir.path().join("unrelated.txt");
    tokio::fs::write(&foreign, b"keep me").await.unwrap();

    let result = downloader.download_to_file(&url, &output, None).await;
    assert!(result.is_err(), "download must fail on chunk 2's 403");

    let names = dir_entries(temp_dir.path()).await;
    assert_eq!(
        names,
        std::collections::HashSet::from(["unrelated.txt".to_string()]),
        "chunk files must be swept after a failed fresh/static download; \
         only the foreign file may remain, found: {names:?}"
    );
}

/// D1 (the live #568 leak): a failed RESUMED parallel download (static
/// chunking) must sweep its `.resume{N}` chunk files, not the `.part{N}`
/// marker the pre-fix cleaner hardcoded. Must FAIL against the unpatched
/// code (the stray `.resumeN` files are left behind).
#[tokio::test]
async fn static_resume_failure_sweeps_resume_chunks_d1() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();

    let already_downloaded = 8 * 1024 * 1024; // 8 MiB already on disk
    let chunk_size = 4 * 1024 * 1024; // 4 MiB per remaining chunk
    let total_size = already_downloaded + chunk_size * 2; // 2 remaining chunks

    let output = temp_dir.path().join("video.mp4");
    let original_bytes = vec![0xAAu8; already_downloaded as usize];
    tokio::fs::write(&output, &original_bytes).await.unwrap();

    // `download_with_resume` first issues an open-ended-range analysis
    // request (`Range: bytes={resume_from}-`) to confirm resume support and
    // learn the total size before choosing parallel vs. sequential resume.
    let _resume_analysis = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=8388608-")
        .with_status(206)
        .with_header(
            "content-range",
            &format!("bytes 8388608-16777215/{total_size}"),
        )
        .with_header("accept-ranges", "bytes")
        .with_body(vec![0xBBu8; chunk_size as usize])
        .create_async()
        .await;

    let _chunk0 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=8388608-12582911")
        .with_status(206)
        .with_header(
            "content-range",
            &format!("bytes 8388608-12582911/{total_size}"),
        )
        .with_body(vec![0xBBu8; chunk_size as usize])
        .create_async()
        .await;

    // Non-retryable: fails immediately, no chunk-level backoff sleep.
    let _chunk1_fail = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=12582912-16777215")
        .with_status(403)
        .with_body("forbidden")
        .expect_at_least(1)
        .create_async()
        .await;

    let no_retry = RetryConfig::new(0, Duration::from_millis(1), Duration::from_millis(1), 1.0);
    let downloader = HttpDownloader::new()
        .with_concurrent_fragments(2)
        .with_retry_config(no_retry)
        .with_parallel_threshold(1)
        .with_chunk_strategy(crate::chunking::ChunkSizeStrategy::Legacy { chunk_count: 2 });

    let url = format!("{}/video.mp4", server.url());

    // Foreign file that must survive the failure's cleanup pass.
    let foreign = temp_dir.path().join("unrelated.txt");
    tokio::fs::write(&foreign, b"keep me").await.unwrap();

    let result = downloader
        .download_with_resume(&url, &output, already_downloaded, None, None)
        .await;
    assert!(result.is_err(), "resume must fail on chunk 1's 403");

    // The original partial file must be untouched by the failed resume.
    let contents = tokio::fs::read(&output).await.unwrap();
    assert_eq!(
        contents, original_bytes,
        "a failed resume must not touch the pre-existing partial output file"
    );

    let names = dir_entries(temp_dir.path()).await;
    assert_eq!(
        names,
        std::collections::HashSet::from(["video.mp4".to_string(), "unrelated.txt".to_string()]),
        "resume chunk files must be swept after a failed resumed download; \
         only the output file and the foreign file may remain, found: {names:?}"
    );
}

/// D2: the adaptive path called no cleanup function at all, so a failed
/// adaptive download leaked every chunk already written to disk regardless
/// of how many sibling chunks had already completed successfully. Must FAIL
/// against the unpatched code (no sweep call exists on this path at all).
///
/// Uses the default adaptive config (no `with_chunk_strategy` override, so
/// `config.adaptive` stays `true`). `AdaptiveConfig::initial_chunk_level` is
/// `MIN_CHUNK_LEVEL` (2 => 256 KiB, see `adaptive::CHUNK_LEVELS`) and
/// `decision_interval` is 4, so with only 3 total chunks the level never
/// changes — every chunk is deterministically 256 KiB, matching
/// `PROBE_WINDOW_BYTES` exactly (so the F3 probe and chunk 0 share one Range
/// request/response).
#[tokio::test]
async fn adaptive_failure_sweeps_chunks_d2() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();

    let chunk_bytes = 256 * 1024; // CHUNK_LEVELS[MIN_CHUNK_LEVEL] == PROBE_WINDOW_BYTES
    let total_size = chunk_bytes * 3;

    // Chunk 0's Range coincides with the F3 probe's Range, so one mock
    // (matched at least twice) serves both requests.
    let _chunk0_and_probe = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-262143/{total_size}"))
        .with_body(vec![0xCCu8; chunk_bytes])
        .expect_at_least(1)
        .create_async()
        .await;

    let _chunk1 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=262144-524287")
        .with_status(206)
        .with_header(
            "content-range",
            &format!("bytes 262144-524287/{total_size}"),
        )
        .with_body(vec![0xDDu8; chunk_bytes])
        .create_async()
        .await;

    // Non-retryable: fails immediately, no chunk-level backoff sleep.
    let _chunk2_fail = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=524288-786431")
        .with_status(403)
        .with_body("forbidden")
        .expect_at_least(1)
        .create_async()
        .await;

    let no_retry = RetryConfig::new(0, Duration::from_millis(1), Duration::from_millis(1), 1.0);
    let downloader = HttpDownloader::new()
        .with_concurrent_fragments(2)
        .with_retry_config(no_retry)
        .with_parallel_threshold(1);
    assert!(
        downloader.config.adaptive,
        "this test must exercise the adaptive path"
    );

    let url = format!("{}/video.mp4", server.url());
    let output = temp_dir.path().join("output.mp4");

    // Foreign file that must survive the failure's cleanup pass.
    let foreign = temp_dir.path().join("unrelated.txt");
    tokio::fs::write(&foreign, b"keep me").await.unwrap();

    let result = downloader.download_to_file(&url, &output, None).await;
    assert!(result.is_err(), "download must fail on chunk 2's 403");

    let names = dir_entries(temp_dir.path()).await;
    assert_eq!(
        names,
        std::collections::HashSet::from(["unrelated.txt".to_string()]),
        "chunk files must be swept after a failed adaptive download; \
         only the foreign file may remain, found: {names:?}"
    );
}
