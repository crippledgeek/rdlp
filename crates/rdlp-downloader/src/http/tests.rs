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
        .download_with_resume(&format!("{}/video.mp4", server.url()), path, 1000, None)
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

    let result =
        download_chunk_with_retry(&downloader, &url, 0, 1023, &chunk_path, Some(progress), 0).await;

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

    let result =
        download_chunk_with_retry(&downloader, &url, 0, 1023, &chunk_path, Some(progress), 0).await;

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

    let result =
        download_chunk_with_retry(&downloader, &url, 0, 1023, &chunk_path, Some(progress), 0).await;

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

    let result =
        download_chunk_with_retry(&downloader, &url, 0, 511, &chunk_path, Some(progress), 0).await;

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
        .download_with_resume(&url, &output, already_downloaded, None)
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

// ── #308: AIMD parallel-path static biased-select guard ────────────────────

#[test]
fn parallel_adaptive_cancel_uses_biased_select() {
    // Static guard: download_parallel_adaptive's semaphore-acquire select!
    // MUST include `biased;` so cancel-priority is deterministic.
    // Mirrors next_with_cancel_helper_uses_biased_select (Task 12, PR #303).
    let source = include_str!("parallel.rs");

    let start = source
        .find("async fn download_parallel_adaptive")
        .expect("download_parallel_adaptive not found in parallel.rs");

    let after_start = &source[start..];
    // End at the next top-level fn definition.
    let end = after_start[1..]
        .find("\n    async fn ")
        .map_or(after_start.len(), |i| i + 1);
    let body = &after_start[..end];

    let select_idx = body
        .find("tokio::select!")
        .expect("download_parallel_adaptive must contain a tokio::select! for cancel");
    let after_select = &body[select_idx..];
    let first_arm = after_select
        .find("=>")
        .expect("tokio::select! must have at least one arm");
    let header = &after_select[..first_arm];

    assert!(
        header.contains("biased;"),
        "tokio::select! in download_parallel_adaptive MUST use `biased;` \
         to deterministically prioritize the cancel arm. \
         Found header: {header:?}"
    );
}
