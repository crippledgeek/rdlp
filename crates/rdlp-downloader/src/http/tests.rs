//! Tests for HTTP downloader module

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

    // Mock HEAD request for size detection (allow multiple calls)
    let mock_head = server
        .mock("HEAD", "/test-video.mp4")
        .with_status(200)
        .with_header("Accept-Ranges", "bytes")
        .with_header("Content-Length", &total_size.to_string())
        .expect_at_least(1)
        .create_async()
        .await;

    // Mock Range request for size fallback detection
    let _mock_size_check = server
        .mock("GET", "/test-video.mp4")
        .match_header("range", "bytes=0-0")
        .with_status(206)
        .with_header("Content-Range", &format!("bytes 0-0/{total_size}"))
        .expect_at_most(1)
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

    // Verify mocks were called appropriately
    mock_head.assert_async().await;

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
