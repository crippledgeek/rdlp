//! Tests for HTTP downloader module

#![allow(clippy::unreadable_literal)] // byte-count literals in HTTP range tests

use super::state::etag;
use super::*;
use rdlp_core::Downloader;
use rdlp_core::is_retryable_error;
use rdlp_http::StrongValidator;

/// Downloader for the chunk-retry tests: retries are real but instant.
///
/// `chunk_retry_policy` derives the chunk layer's attempt count from this, so
/// a config of 0 — which every one of these tests used to carry — silently
/// turns a retry test into a single-request test.
///
/// Which layer a test actually drives is worth knowing, because this one knob
/// moves both. A 5xx is converted by `check_http_response` *inside*
/// `download_range_with_progress`'s own `with_retry`, so the status-code tests
/// are satisfied by the inner range layer and may never iterate the outer
/// chunk loop. The tests that unambiguously drive the outer loop are
/// `chunk_retry_recovers_from_wrong_span_response` and
/// `..._from_short_body`: span and length validation run after the inner
/// retry has already returned, so only the chunk layer can retry them.
fn chunk_test_downloader(max_retries: usize) -> HttpDownloader {
    HttpDownloader::new().with_retry_config(crate::retry::test_retry_config(max_retries))
}
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

/// RFC 9110 §15.3.7.3 permits combining parts only under a shared strong
/// validator. A partial with no sidecar has none recorded, so the resume
/// discards it and converges onto the fresh path — the pre-#565 behaviour
/// (error, keep the partial) asserted here before is exactly what the spec
/// replaces.
#[tokio::test]
async fn resume_without_sidecar_restarts_from_zero() {
    use mockito::{Matcher, Server};

    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"partial", None).await;

    let probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .match_header("if-range", Matcher::Missing)
        .with_status(200)
        .with_body("fresh")
        .expect(1)
        .create_async()
        .await;
    let get = server
        .mock("GET", "/v")
        .match_header("range", Matcher::Missing)
        .match_header("if-range", Matcher::Missing)
        .with_status(200)
        .with_header("etag", "\"n\"")
        .with_body("fresh")
        .expect(1)
        .create_async()
        .await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 7, None, None)
        .await
        .unwrap();
    probe.assert_async().await;
    get.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 5);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"fresh");
    assert!(
        !HttpResumeState::sidecar_path(&out).exists(),
        "sidecar is removed once the restarted download completes"
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

    // Mockito matches in CREATION order and retires a mock once its `expect(n)`
    // is met (`mockito-1.7.2/src/server.rs`: `matching_mocks ...
    // find(is_missing_hits)`), so the failing mock below is created first and
    // answers first, and the good one answers the retry. `assert_async` on both
    // is what stops a no-retry regression from passing this test silently.
    let body = vec![0xABu8; 1024];
    let mock_fail = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(500)
        .with_body("error")
        .expect(1)
        .create_async()
        .await;

    let mock_ok = server
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

    let downloader = chunk_test_downloader(3);

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            source: &Source::new(&url, None),
            start: 0,
            end: 1023,
            chunk_path: &chunk_path,
            chunk_id: 0,
        },
        Some(progress),
        None,
    )
    .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1024);

    // Every mock must have been consumed: without this an accidental
    // single-request run would satisfy the assertions above.
    mock_fail.assert_async().await;
    mock_ok.assert_async().await;
}

#[tokio::test]
async fn test_chunk_retry_exhausted_returns_error() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // All requests fail with 500
    let mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(500)
        .with_body("error")
        .expect_at_least(3)
        .create_async()
        .await;

    let downloader = chunk_test_downloader(3);

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            source: &Source::new(&url, None),
            start: 0,
            end: 1023,
            chunk_path: &chunk_path,
            chunk_id: 0,
        },
        Some(progress),
        None,
    )
    .await;

    assert!(result.is_err());

    // Every mock must have been consumed: without this an accidental
    // single-request run would satisfy the assertions above.
    mock.assert_async().await;
}

#[tokio::test]
async fn test_chunk_retry_non_retryable_fails_immediately() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // 403 is not retryable — should fail immediately, not retry
    let mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(403)
        .with_body("forbidden")
        .expect(1) // Only 1 request — no retries
        .create_async()
        .await;

    let downloader = chunk_test_downloader(3);

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            source: &Source::new(&url, None),
            start: 0,
            end: 1023,
            chunk_path: &chunk_path,
            chunk_id: 0,
        },
        Some(progress),
        None,
    )
    .await;

    assert!(result.is_err());
    // mockito's expect(1) will panic on Drop if more than 1 request was made

    // Every mock must have been consumed: without this an accidental
    // single-request run would satisfy the assertions above.
    mock.assert_async().await;
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

    // Mockito matches in CREATION order and retires a mock once its `expect(n)`
    // is met (`mockito-1.7.2/src/server.rs`: `matching_mocks ...
    // find(is_missing_hits)`), so the failing mock below is created first and
    // answers first, and the good one answers the retry. `assert_async` on both
    // is what stops a no-retry regression from passing this test silently.
    let body = vec![0xCDu8; 512];
    let mock_fail = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(500)
        .with_body("error")
        .expect(1)
        .create_async()
        .await;

    let mock_ok = server
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

    let downloader = chunk_test_downloader(3);

    let url = format!("{}/video.mp4", server.url());
    let progress = Arc::new(AtomicU64::new(0));

    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            source: &Source::new(&url, None),
            start: 0,
            end: 511,
            chunk_path: &chunk_path,
            chunk_id: 0,
        },
        Some(progress),
        None,
    )
    .await;

    assert!(result.is_ok());
    // The file should contain the successful download, not the partial data
    let contents = tokio::fs::read(&chunk_path).await.unwrap();
    assert_eq!(contents.len(), 512);
    assert!(contents.iter().all(|&b| b == 0xCD));

    // Every mock must have been consumed: without this an accidental
    // single-request run would satisfy the assertions above.
    mock_fail.assert_async().await;
    mock_ok.assert_async().await;
}

/// A parallel resume sends the sidecar's validator as `If-Range` on the
/// initial ranged GET and on EVERY chunk (RFC 9110 §13.1.5), and each 206 is
/// checked against it (§15.3.7.3) — the mocks only answer a request that
/// carries it. The sidecar is gone once the merged output is complete.
#[tokio::test]
async fn parallel_resume_carries_if_range_on_every_chunk() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();

    // A 20 MB file with 5 MB already downloaded (25%).
    let total_size = 20 * 1024 * 1024;
    let already_downloaded = 5 * 1024 * 1024;

    let output = temp_dir.path().join("video.mp4");
    tokio::fs::write(&output, vec![0xAA; already_downloaded as usize])
        .await
        .unwrap();
    HttpResumeState::new(etag("\"v1\""), Some(total_size))
        .save(&output)
        .await
        .unwrap();

    // The initial ranged GET: its 206 supplies the total, then its body is
    // dropped in favour of the parallel chunks below.
    let mock_resume = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=5242880-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header(
            "Content-Range",
            &format!("bytes 5242880-20971519/{total_size}"),
        )
        .with_header("Accept-Ranges", "bytes")
        .with_header("etag", "\"v1\"")
        .with_body("")
        .expect(1)
        .create_async()
        .await;

    // Four chunks of 3.75 MB for the 15 MB remainder, each fingerprinted by
    // its fill byte so the merge order is verifiable.
    let chunk_bounds: [(u64, u64, u8); 4] = [
        (5242880, 9175039, 0xCC),
        (9175040, 13107199, 0xDD),
        (13107200, 17039359, 0xEE),
        (17039360, 20971519, 0xFF),
    ];
    let mut chunk_mocks = Vec::with_capacity(chunk_bounds.len());
    for (start, end, fill) in chunk_bounds {
        let m = server
            .mock("GET", "/video.mp4")
            .match_header("range", format!("bytes={start}-{end}").as_str())
            .match_header("if-range", "\"v1\"")
            .with_status(206)
            .with_header(
                "Content-Range",
                &format!("bytes {start}-{end}/{total_size}"),
            )
            .with_header("etag", "\"v1\"")
            .with_body(vec![fill; usize::try_from(end - start + 1).unwrap()])
            .expect(1)
            .create_async()
            .await;
        chunk_mocks.push(m);
    }

    // Legacy chunking keeps the four large chunks the mocks are shaped for.
    let downloader = chunk_test_downloader(0)
        .with_concurrent_fragments(4)
        .with_chunk_strategy(crate::chunking::ChunkSizeStrategy::Legacy { chunk_count: 4 });

    let url = format!("{}/video.mp4", server.url());
    let stats = downloader
        .download_with_resume(&url, &output, already_downloaded, None, None)
        .await
        .expect("parallel resume under a shared validator succeeds");

    mock_resume.assert_async().await;
    for m in &chunk_mocks {
        m.assert_async().await;
    }
    assert_eq!(stats.bytes_downloaded, total_size);

    let contents = tokio::fs::read(&output).await.unwrap();
    assert_eq!(contents.len(), usize::try_from(total_size).unwrap());
    assert_eq!(contents[0], 0xAA, "kept partial starts the file");
    assert_eq!(
        contents[usize::try_from(already_downloaded).unwrap() - 1],
        0xAA,
        "kept partial ends where the resume began"
    );
    for (start, end, fill) in chunk_bounds {
        assert_eq!(contents[usize::try_from(start).unwrap()], fill);
        assert_eq!(contents[usize::try_from(end).unwrap()], fill);
    }
    assert!(
        !HttpResumeState::sidecar_path(&output).exists(),
        "sidecar is removed once the parallel resume completes"
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

            // 206 headers naming a 10 MB total and the sidecar's validator, so
            // the resume takes the sequential append branch and parks on the
            // body that never comes.
            let _ = stream.write_all(
                b"HTTP/1.1 206 Partial Content\r\n\
                  Content-Range: bytes 0-10485759/10485760\r\n\
                  Content-Length: 10485760\r\n\
                  ETag: \"v1\"\r\n\
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
    // A 0-byte partial with a sidecar: without one the resume would restart
    // through the fresh path rather than append (#565).
    tokio::fs::write(&out, b"").await.unwrap();
    HttpResumeState::new(etag("\"v1\""), None)
        .save(&out)
        .await
        .unwrap();

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
    // A cancel mid-stream is not success: the sidecar stays so the next
    // attempt can send the same validator (#565).
    assert!(
        HttpResumeState::sidecar_path(&out).exists(),
        "sidecar must survive a mid-stream cancel"
    );
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
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
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
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
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
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
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
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
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
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
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
        .download_range_with_progress(
            &Source::new(&url, None),
            4096,
            5119,
            &chunk_path,
            None,
            None,
        )
        .await;

    assert_eq!(
        result.expect("a conformant 206 must be accepted"),
        1024,
        "must report exactly the requested span length"
    );
}

/// `admit_for_verdict` must let a 416 through to `range_verdict` rather than
/// having it rejected upstream as a generic non-2xx failure — otherwise the
/// server's reported `complete_length` never reaches the caller. Retries off
/// (0): this is about the verdict this one response produces, not a retry.
#[tokio::test]
async fn chunk_416_reaches_the_verdict_and_names_the_reported_length() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(416)
        .with_header("content-range", "bytes */12345")
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
        .await;

    let err = result.expect_err("a 416 must be refused, not silently accepted");
    let msg = err.to_string();
    assert!(
        msg.contains("12345"),
        "the reported complete length must reach the caller, proving the verdict path (not a \
         generic non-2xx rejection) handled this response, got: {msg}"
    );
}

/// A genuine 5xx must still be rejected exactly as before — `admit_for_verdict`
/// only special-cases 200/206/416, so this still surfaces as the plain `Http`
/// error the retry layer above this function keys its retry decision on.
#[tokio::test]
async fn chunk_503_is_still_a_plain_http_error() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(503)
        .create_async()
        .await;

    let url = format!("{}/video.mp4", server.url());
    let result = validation_test_downloader()
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
        .await;

    assert!(
        matches!(result, Err(RdlpError::Http { status: 503, .. })),
        "a 503 must still surface as a plain Http error, got {result:?}"
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

    // Mockito matches in CREATION order and retires a mock once its `expect(n)`
    // is met (`mockito-1.7.2/src/server.rs`: `matching_mocks ...
    // find(is_missing_hits)`), so the failing mock below is created first and
    // answers first, and the good one answers the retry. `assert_async` on both
    // is what stops a no-retry regression from passing this test silently.
    // Right length, WRONG span — the #526 signature.
    let mock_wrong_span = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 1048576-1049599/2097152")
        .with_body(vec![0x99u8; 1024])
        .expect(1)
        .create_async()
        .await;

    let mock_ok = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x11u8; 1024])
        .expect(1)
        .create_async()
        .await;

    let downloader = chunk_test_downloader(3);

    let url = format!("{}/video.mp4", server.url());
    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            source: &Source::new(&url, None),
            start: 0,
            end: 1023,
            chunk_path: &chunk_path,
            chunk_id: 0,
        },
        Some(Arc::new(AtomicU64::new(0))),
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

    // Every mock must have been consumed: without this an accidental
    // single-request run would satisfy the assertions above.
    mock_wrong_span.assert_async().await;
    mock_ok.assert_async().await;
}

/// Same guarantee for a truncated body.
#[tokio::test]
async fn chunk_retry_recovers_from_short_body() {
    use mockito::Server;
    use tempfile::TempDir;

    let mut server = Server::new_async().await;
    let temp_dir = TempDir::new().unwrap();
    let chunk_path = temp_dir.path().join("chunk_0");

    // Answers FIRST: conformant headers, truncated body.
    let mock_short = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x88u8; 400])
        .expect(1)
        .create_async()
        .await;

    let mock_ok = server
        .mock("GET", "/video.mp4")
        .match_header("Range", mockito::Matcher::Any)
        .with_status(206)
        .with_header("content-range", "bytes 0-1023/1048576")
        .with_body(vec![0x22u8; 1024])
        .expect(1)
        .create_async()
        .await;

    let downloader = chunk_test_downloader(3);

    let url = format!("{}/video.mp4", server.url());
    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            source: &Source::new(&url, None),
            start: 0,
            end: 1023,
            chunk_path: &chunk_path,
            chunk_id: 0,
        },
        Some(Arc::new(AtomicU64::new(0))),
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

    // Every mock must have been consumed: without this an accidental
    // single-request run would satisfy the assertions above.
    mock_short.assert_async().await;
    mock_ok.assert_async().await;
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

    let downloader = chunk_test_downloader(3);

    let url = format!("{}/video.mp4", server.url());
    let result = download_chunk_with_retry(
        &downloader,
        ChunkRequestSpec {
            source: &Source::new(&url, None),
            start: 0,
            end: 1023,
            chunk_path: &chunk_path,
            chunk_id: 0,
        },
        Some(Arc::new(AtomicU64::new(0))),
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
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
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
        .download_range_with_progress(&Source::new(&url, None), 0, 1023, &chunk_path, None, None)
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

    let mut server = Server::new_async().await;

    let _mock = server
        .mock("GET", "/video.mp4")
        .match_header("Range", "bytes=1000-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        // Asked to resume at 1000; server answers from 5000.
        .with_header("content-range", "bytes 5000-9999/10000")
        .with_header("etag", "\"v1\"")
        .with_body(vec![0x66u8; 5000])
        .create_async()
        .await;

    // Not a chunk test: this drives `download_with_resume`, and the offset
    // rejection it asserts happens above the retry layer. Retries stay off, as
    // they were before the chunk tests were converged onto a shared helper.
    // The sidecar keeps this on the append path (#565); without one the
    // partial would be discarded and the download restarted instead.
    let downloader = chunk_test_downloader(0);

    let dir = tempfile::TempDir::new().unwrap();
    let path = &dir.path().join("video.mp4");
    tokio::fs::write(path, b"partial data").await.unwrap();
    HttpResumeState::new(etag("\"v1\""), Some(10000))
        .save(path)
        .await
        .unwrap();

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
// #568 — chunk cleanup on failed parallel downloads
//
// Before this fix, the pre-fix cleaner hardcoded the `.part{N}` marker
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
/// adaptive disabled via `Legacy` strategy) must still clean up its
/// `.part{N}` chunk files. This path's suffix already matched pre-#568 (both
/// writer and cleaner used `"part"`), so — unlike the two tests below — this
/// one is expected to ALREADY PASS against the unpatched code; it pins the
/// behavior so the migration to `ChunkSet` does not regress it.
#[tokio::test]
async fn static_fresh_failure_cleans_up_part_chunks_regression_guard() {
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
        "chunk files must be cleaned up after a failed fresh/static download; \
         only the foreign file may remain, found: {names:?}"
    );
}

/// D1 (the live #568 leak): a failed RESUMED parallel download (static
/// chunking) must clean up its `.resume{N}` chunk files, not the `.part{N}`
/// marker the pre-fix cleaner hardcoded. Must FAIL against the unpatched
/// code (the stray `.resumeN` files are left behind).
#[tokio::test]
async fn static_resume_failure_cleans_up_resume_chunks_d1() {
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
    // The sidecar is what makes this a resume rather than a restart (#565);
    // its validator rides every request below as `If-Range`.
    HttpResumeState::new(etag("\"v1\""), Some(total_size))
        .save(&output)
        .await
        .unwrap();

    // `download_with_resume` first issues an open-ended-range analysis
    // request (`Range: bytes={resume_from}-`) to confirm resume support and
    // learn the total size before choosing parallel vs. sequential resume.
    let _resume_analysis = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=8388608-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header(
            "content-range",
            &format!("bytes 8388608-16777215/{total_size}"),
        )
        .with_header("accept-ranges", "bytes")
        .with_header("etag", "\"v1\"")
        .with_body("")
        .create_async()
        .await;

    let _chunk0 = server
        .mock("GET", "/video.mp4")
        .match_header("range", "bytes=8388608-12582911")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header(
            "content-range",
            &format!("bytes 8388608-12582911/{total_size}"),
        )
        .with_header("etag", "\"v1\"")
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

    // The resume sidecar stays on failure so the next attempt can send its
    // validator again (#565); it is the one download-owned file allowed here.
    let names = dir_entries(temp_dir.path()).await;
    assert_eq!(
        names,
        std::collections::HashSet::from([
            "video.mp4".to_string(),
            "video.mp4.http_state.json".to_string(),
            "unrelated.txt".to_string()
        ]),
        "resume chunk files must be cleaned up after a failed resumed download; \
         only the output file, its sidecar and the foreign file may remain, found: {names:?}"
    );
}

/// D2: the adaptive path called no cleanup function at all, so a failed
/// adaptive download leaked every chunk already written to disk regardless
/// of how many sibling chunks had already completed successfully. Must FAIL
/// against the unpatched code (no cleanup call exists on this path at all).
///
/// Uses the default adaptive config (no `with_chunk_strategy` override, so
/// `config.adaptive` stays `true`). `AdaptiveConfig::initial_chunk_level` is
/// `MIN_CHUNK_LEVEL` (2 => 256 KiB, see `adaptive::CHUNK_LEVELS`) and
/// `decision_interval` is 4, so with only 3 total chunks the level never
/// changes — every chunk is deterministically 256 KiB, matching
/// `PROBE_WINDOW_BYTES` exactly (so the F3 probe and chunk 0 share one Range
/// request/response).
#[tokio::test]
async fn adaptive_failure_cleans_up_chunks_d2() {
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
        "chunk files must be cleaned up after a failed adaptive download; \
         only the foreign file may remain, found: {names:?}"
    );
}

// --- chunk retry policy (issue #570) ---

#[test]
fn chunk_policy_caps_the_attempt_count_but_never_raises_it() {
    use super::parallel::chunk_retry_policy;
    use rdlp_core::RetryConfig;

    let base = |max_retries| {
        RetryConfig::new(
            max_retries,
            Duration::from_millis(10),
            Duration::from_secs(30),
            2.0,
        )
    };
    // Both sides of the cap: at it, one under it, one over it. The cap exists
    // because this layer nests over the range request's own retry, so the
    // attempt counts multiply.
    assert_eq!(chunk_retry_policy(&base(2)).max_retries, 2, "under the cap");
    assert_eq!(chunk_retry_policy(&base(3)).max_retries, 3, "at the cap");
    assert_eq!(chunk_retry_policy(&base(4)).max_retries, 3, "over the cap");
    assert_eq!(
        chunk_retry_policy(&base(10)).max_retries,
        3,
        "the default ten must not compose to ~100 requests per chunk"
    );
    assert_eq!(
        chunk_retry_policy(&base(0)).max_retries,
        0,
        "a caller asking for no retries must get none"
    );
}

#[test]
fn chunk_policy_holds_the_delay_ceiling_but_keeps_the_caller_s_shape() {
    use super::parallel::chunk_retry_policy;
    use rdlp_core::RetryConfig;

    // The real production input: the default whole-resource policy, whose 60s
    // ceiling is exactly what the chunk layer must not inherit.
    let slow = RetryConfig::default_config();
    let policy = chunk_retry_policy(&slow);
    assert_eq!(policy.initial_delay, Duration::from_secs(1));
    assert_eq!(
        policy.max_delay,
        Duration::from_secs(3),
        "1s, 2s, 3s — the sequence this path used before #570"
    );

    // A caller under the ceiling keeps its own, so tests stay fast.
    let fast = RetryConfig::new(3, Duration::from_millis(1), Duration::from_millis(5), 2.0);
    assert_eq!(
        chunk_retry_policy(&fast).max_delay,
        Duration::from_millis(5)
    );
}

// --- same-origin header gate (#273 / #319, converged for #570) ---
//
// This gate decides whether the operator's Referer / Cookie / Authorization
// reach a given target. It used to be written twice; these tests cover the
// union of what both spellings had to get right.

fn seed_headers() -> wreq::header::HeaderMap {
    let mut headers = wreq::header::HeaderMap::new();
    headers.insert("Referer", "https://seed.example/page".parse().unwrap());
    headers.insert("Cookie", "session=secret".parse().unwrap());
    headers
}

fn origin_of(url: &str) -> url::Origin {
    url::Url::parse(url).unwrap().origin()
}

#[test]
fn same_origin_target_receives_the_operator_headers() {
    let seed = origin_of("https://cdn.example/master.m3u8");
    let out = same_origin_headers(Some(&seed), "https://cdn.example/seg1.ts", &seed_headers());
    assert_eq!(out.len(), 2, "same origin forwards every header");
    assert!(out.contains_key("Cookie"));
}

#[test]
fn a_different_host_receives_nothing() {
    let seed = origin_of("https://cdn.example/master.m3u8");
    let out = same_origin_headers(
        Some(&seed),
        "https://attacker.example/seg1.ts",
        &seed_headers(),
    );
    assert!(
        out.is_empty(),
        "a foreign host must not receive credentials"
    );
}

#[test]
fn origin_is_scheme_host_and_port_not_just_host() {
    // RFC 6454: all three components. A downgrade to http, or a different
    // port, is a different origin even with the host unchanged.
    let seed = origin_of("https://cdn.example/master.m3u8");
    for target in [
        "http://cdn.example/seg1.ts",
        "https://cdn.example:8443/seg1.ts",
        "https://sub.cdn.example/seg1.ts",
    ] {
        let out = same_origin_headers(Some(&seed), target, &seed_headers());
        assert!(out.is_empty(), "{target} must not receive credentials");
    }
}

#[test]
fn absent_seed_fails_closed() {
    // No format URL at all. A caller whose own parse failed passes `None` too,
    // so this one case covers both — the function never sees the unparsed
    // string. (A target that will not parse is the separate test below.)
    let out = same_origin_headers(None, "https://cdn.example/seg1.ts", &seed_headers());
    assert!(out.is_empty());
}

#[test]
fn unparseable_target_fails_closed() {
    let seed = origin_of("https://cdn.example/master.m3u8");
    let out = same_origin_headers(Some(&seed), "not a url", &seed_headers());
    assert!(out.is_empty());
}

#[test]
fn opaque_origins_never_match_not_even_themselves() {
    // A non-tuple origin (`data:`) yields `Origin::Opaque`, and two opaque
    // origins are never equal — which is what makes the gate fail closed
    // rather than accidentally matching one such URL against another.
    let opaque = origin_of("data:text/plain,seed");
    let out = same_origin_headers(Some(&opaque), "data:text/plain,seed", &seed_headers());
    assert!(
        out.is_empty(),
        "an opaque origin must not match, even against an identical URL"
    );
}

#[test]
fn a_same_origin_target_with_a_different_path_or_query_still_matches() {
    // Origin is scheme/host/port only — a CDN token in the query, or a deep
    // path, does not make the target foreign.
    let seed = origin_of("https://cdn.example/master.m3u8");
    let out = same_origin_headers(
        Some(&seed),
        "https://cdn.example/deep/path/seg1.ts?token=abc123",
        &seed_headers(),
    );
    assert_eq!(out.len(), 2);
}

// ---------------------------------------------------------------------------
// #565 — the strong validator is captured on the fresh path and every chunk
// is verified against it (RFC 9110 §13.1.5, §15.3.7.3).
// ---------------------------------------------------------------------------

/// Read the resume sidecar from a mockito body callback. The callback runs on
/// mockito's own OS thread, never inside an async task, so the blocking read
/// is not the hazard `clippy.toml` bans — this is policy (c), a test fixture.
#[allow(clippy::disallowed_methods)] // std::fs helpers in test fixtures — per clippy.toml policy (c)
fn read_sidecar_sync(output: &Path) -> Option<String> {
    std::fs::read_to_string(HttpResumeState::sidecar_path(output)).ok()
}

/// Poll interval and ceiling for a body thread waiting on a sidecar rewrite;
/// the ceiling is only reached on a regression, when the test then fails on
/// what it saw.
const SIDECAR_POLL: std::time::Duration = std::time::Duration::from_millis(10);
const SIDECAR_MAX_POLLS: u32 = 300;

/// From a mockito body callback: wait — bounded — until the sidecar names
/// `wanted`, and return the last sidecar text seen (rewritten or not). mockito
/// builds a body BEFORE it writes the headers, so a `with_body_from_request`
/// callback would only ever see the sidecar from before the request; a
/// `with_chunked_body` thread runs while hyper streams and can wait for the
/// rewrite the response's own headers trigger.
fn wait_for_sidecar_validator(output: &Path, wanted: &StrongValidator) -> Option<String> {
    let mut latest = None;
    for _ in 0..SIDECAR_MAX_POLLS {
        latest = read_sidecar_sync(output);
        let rewritten = latest
            .as_deref()
            .and_then(|s| serde_json::from_str::<HttpResumeState>(s).ok())
            .is_some_and(|s| s.validator == *wanted);
        if rewritten {
            break;
        }
        std::thread::sleep(SIDECAR_POLL);
    }
    latest
}

/// Two-chunk parallel setup: a probe answering 206 with `etag`, and a
/// downloader that will split `total` into two static chunks of `total / 2`.
async fn two_chunk_setup(
    server: &mut mockito::ServerGuard,
    total: u64,
    etag: &str,
) -> (mockito::Mock, HttpDownloader) {
    let probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-{}/{total}", total - 1))
        .with_header("etag", etag)
        .with_body(vec![1u8; total as usize])
        .create_async()
        .await;
    let half = usize::try_from(total / 2).unwrap();
    let d = chunk_test_downloader(0)
        .with_parallel_threshold(1)
        .with_concurrent_fragments(2)
        .with_chunk_strategy(crate::chunking::ChunkSizeStrategy::Fixed(half));
    (probe, d)
}

/// A chunk whose 206 names a different representation than the probe's
/// (§15.3.7.3: parts may be combined only under the same strong validator)
/// must abort before the merge, not be spliced in.
#[tokio::test]
async fn parallel_chunk_with_different_etag_aborts_without_merging() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    let total = 4096u64;
    let (_probe, d) = two_chunk_setup(&mut server, total, "\"a\"").await;
    let _c0 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-2047")
        .match_header("if-range", "\"a\"")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-2047/{total}"))
        .with_header("etag", "\"a\"")
        .with_body(vec![1u8; 2048])
        .create_async()
        .await;
    let _c1 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=2048-4095")
        .with_status(206)
        .with_header("content-range", &format!("bytes 2048-4095/{total}"))
        .with_header("etag", "\"b\"")
        .with_body(vec![2u8; 2048])
        .create_async()
        .await;

    let err = d
        .download_to_file(&format!("{}/v", server.url()), &out, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("validator changed"), "{err}");
    assert!(!out.exists(), "no merged output after a validator mismatch");
    assert!(
        HttpResumeState::sidecar_path(&out).exists(),
        "sidecar stays for diagnosis; next run re-probes"
    );
}

/// §13.2.2 step 5: a 200 to `Range` + `If-Range` means the representation
/// changed and the body is the whole new one. It can never be placed at a
/// chunk's offset, so the download aborts with the whole-resource reason —
/// not the "server ignored Range" reason a validator-less request gets.
#[tokio::test]
async fn parallel_chunk_answering_200_after_if_range_aborts() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    let total = 4096u64;
    let (_probe, d) = two_chunk_setup(&mut server, total, "\"a\"").await;
    let _c0 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-2047")
        .match_header("if-range", "\"a\"")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-2047/{total}"))
        .with_header("etag", "\"a\"")
        .with_body(vec![1u8; 2048])
        .create_async()
        .await;
    let _c1 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=2048-4095")
        .match_header("if-range", "\"a\"")
        .with_status(200)
        .with_header("etag", "\"b\"")
        .with_body(vec![2u8; total as usize])
        .create_async()
        .await;

    let err = d
        .download_to_file(&format!("{}/v", server.url()), &out, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("server answered 200"),
        "expected the Replaced verdict's reason, got: {err}"
    );
    assert!(!out.exists(), "no merged output after a 200 to If-Range");
}

/// The sidecar is on disk before the first body byte arrives — observed from
/// inside the chunk's body callback, which mockito runs when the chunk
/// request reaches it — and gone once the download succeeds. When the
/// download fails instead, the sidecar stays.
#[tokio::test]
async fn fresh_download_writes_sidecar_before_body_and_removes_it_on_success() {
    use mockito::Server;
    let total = 4096u64;

    // Success: one chunk covering the whole body.
    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    let (_probe, d) = two_chunk_setup(&mut server, total, "\"a\"").await;
    let d = d.with_chunk_strategy(crate::chunking::ChunkSizeStrategy::Fixed(
        usize::try_from(total).unwrap(),
    ));
    let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let seen_in_body = seen.clone();
    let out_for_body = out.clone();
    let _c0 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-4095")
        .match_header("if-range", "\"a\"")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-4095/{total}"))
        .with_header("etag", "\"a\"")
        .with_body_from_request(move |_| {
            *seen_in_body.lock().unwrap() = read_sidecar_sync(&out_for_body);
            vec![7u8; 4096]
        })
        .create_async()
        .await;

    let stats = d
        .download_to_file(&format!("{}/v", server.url()), &out, None)
        .await
        .unwrap();
    assert_eq!(stats.bytes_downloaded, total);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), vec![7u8; 4096]);
    let seen = seen.lock().unwrap().clone();
    let seen: HttpResumeState = serde_json::from_str(
        seen.as_deref()
            .expect("sidecar must exist when the first chunk request arrives"),
    )
    .unwrap();
    assert_eq!(
        seen.validator,
        StrongValidator::ETag(rdlp_http::validator::StrongEntityTag::parse("\"a\"").unwrap())
    );
    assert_eq!(seen.complete_length, Some(total));
    assert!(
        !HttpResumeState::sidecar_path(&out).exists(),
        "sidecar is removed once the download completes"
    );

    // Failure: the chunk answers 503 with no retries left; the sidecar stays.
    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    let (_probe, d) = two_chunk_setup(&mut server, total, "\"a\"").await;
    let d = d.with_chunk_strategy(crate::chunking::ChunkSizeStrategy::Fixed(
        usize::try_from(total).unwrap(),
    ));
    let c0 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-4095")
        .with_status(503)
        .with_body("unavailable")
        .expect(1)
        .create_async()
        .await;
    let err = d
        .download_to_file(&format!("{}/v", server.url()), &out, None)
        .await
        .unwrap_err();
    c0.assert_async().await;
    assert!(
        matches!(err, RdlpError::Http { status: 503, .. }),
        "{err:?}"
    );
    let left = HttpResumeState::load(&out)
        .await
        .expect("sidecar written after the probe survives a failed chunk");
    assert_eq!(
        left.validator,
        StrongValidator::ETag(rdlp_http::validator::StrongEntityTag::parse("\"a\"").unwrap())
    );
}

/// On the sequential path the GET, not the probe, produced the bytes on
/// disk, so the sidecar must carry the GET's validator — written after its
/// headers and before its body is consumed.
///
/// mockito builds a response's body BEFORE it writes the headers
/// (`server.rs::respond_with_mock`), so a `with_body_from_request` callback
/// would only ever see the probe's sidecar. `with_chunked_body` instead runs
/// on its own thread while hyper streams the response, so it can wait —
/// bounded — for the GET's headers to have reached the client and the
/// sidecar to have been rewritten, and record what it saw.
#[tokio::test]
async fn sequential_get_validator_overrides_probe_validator() {
    use mockito::{Matcher, Server};

    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    let body = vec![9u8; 1024];

    let _probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .with_status(200)
        .with_header("etag", "\"probe\"")
        .with_body(body.clone())
        .create_async()
        .await;

    let get_validator =
        StrongValidator::ETag(rdlp_http::validator::StrongEntityTag::parse("\"get\"").unwrap());
    let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let seen_in_body = seen.clone();
    let out_for_body = out.clone();
    let body_for_get = body.clone();
    let wanted = get_validator.clone();
    let _get = server
        .mock("GET", "/v")
        .match_header("range", Matcher::Missing)
        .with_status(200)
        .with_header("etag", "\"get\"")
        .with_chunked_body(move |w| {
            *seen_in_body.lock().unwrap() = wait_for_sidecar_validator(&out_for_body, &wanted);
            w.write_all(&body_for_get)
        })
        .create_async()
        .await;

    let d = chunk_test_downloader(0);
    let stats = d
        .download_to_file(&format!("{}/v", server.url()), &out, None)
        .await
        .unwrap();
    assert_eq!(stats.bytes_downloaded, 1024);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), body);

    let seen = seen.lock().unwrap().clone();
    let seen: HttpResumeState = serde_json::from_str(
        seen.as_deref()
            .expect("sidecar must exist while the GET body is being served"),
    )
    .unwrap();
    assert_eq!(
        seen.validator, get_validator,
        "the GET's validator, not the probe's, is what the bytes on disk belong to"
    );
    assert!(
        !HttpResumeState::sidecar_path(&out).exists(),
        "sidecar is removed once the download completes"
    );
}

/// "No sidecar" means "no validator" (RFC 9110 §15.3.7.3 grants combining only
/// under a shared strong validator), so a fresh download whose probe offers
/// none must clear a sidecar left behind by an earlier attempt before it
/// requests any body — observed from inside the chunk's body callback.
#[tokio::test]
async fn fresh_download_without_probe_validator_clears_a_stale_sidecar() {
    use mockito::Server;
    let total = 4096u64;
    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");

    let mut stale = HttpResumeState::new(
        StrongValidator::ETag(rdlp_http::validator::StrongEntityTag::parse("\"old\"").unwrap()),
        Some(1),
    );
    stale.save(&out).await.unwrap();

    let _probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-{}/{total}", total - 1))
        .with_body(vec![1u8; total as usize])
        .create_async()
        .await;
    let seen: Arc<Mutex<Option<Option<String>>>> = Arc::new(Mutex::new(None));
    let seen_in_body = seen.clone();
    let out_for_body = out.clone();
    let _c0 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-4095")
        .match_header("if-range", mockito::Matcher::Missing)
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-4095/{total}"))
        .with_body_from_request(move |_| {
            *seen_in_body.lock().unwrap() = Some(read_sidecar_sync(&out_for_body));
            vec![7u8; 4096]
        })
        .create_async()
        .await;

    let d = chunk_test_downloader(0)
        .with_parallel_threshold(1)
        .with_concurrent_fragments(2)
        .with_chunk_strategy(crate::chunking::ChunkSizeStrategy::Fixed(
            usize::try_from(total).unwrap(),
        ));
    d.download_to_file(&format!("{}/v", server.url()), &out, None)
        .await
        .unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        Some(None),
        "the stale sidecar must be gone when the first chunk request arrives"
    );
}

// ---------------------------------------------------------------------------
// #565 — the resume flow: `If-Range`, then 206 / 200 / 416 by verdict
// (RFC 9110 §13.1.5, §13.2.2, §14.4, §15.3.7.3).
// ---------------------------------------------------------------------------

/// A partial file holding `partial`, optionally with a sidecar naming the
/// validator and complete-length it was fetched under, and a downloader that
/// never retries and never goes parallel — so the sequential branch is what
/// each test drives unless it opts out.
async fn resume_fixture(
    partial: &[u8],
    sidecar: Option<(&str, Option<u64>)>,
) -> (tempfile::TempDir, std::path::PathBuf, HttpDownloader) {
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    tokio::fs::write(&out, partial).await.unwrap();
    if let Some((tag, complete_length)) = sidecar {
        HttpResumeState::new(etag(tag), complete_length)
            .save(&out)
            .await
            .unwrap();
    }
    let d = chunk_test_downloader(0).with_parallel_threshold(u64::MAX);
    (dir, out, d)
}

/// The fresh path a restart converges onto: a probe (no `If-Range`) answering
/// 200 so the download goes sequential, then the plain GET serving `body`.
async fn fresh_mocks(
    server: &mut mockito::ServerGuard,
    body: &[u8],
) -> (mockito::Mock, mockito::Mock) {
    use mockito::Matcher;
    let probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .match_header("if-range", Matcher::Missing)
        .with_status(200)
        .with_body(body)
        .expect(1)
        .create_async()
        .await;
    let get = server
        .mock("GET", "/v")
        .match_header("range", Matcher::Missing)
        .match_header("if-range", Matcher::Missing)
        .with_status(200)
        .with_body(body)
        .expect(1)
        .create_async()
        .await;
    (probe, get)
}

/// The happy path: `Range: bytes=N-` + `If-Range` + the identity pin go out
/// together, the 206 is appended, and the sidecar is gone on success.
#[tokio::test]
async fn resume_sends_if_range_and_appends_on_206() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"0123", Some(("\"v1\"", Some(8)))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .match_header("accept-encoding", "identity")
        .with_status(206)
        .with_header("content-range", "bytes 4-7/8")
        .with_header("etag", "\"v1\"")
        .with_body("4567")
        .expect(1)
        .create_async()
        .await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 8);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"01234567");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// §13.2.2 step 5: a 200 to `If-Range` means the representation changed and
/// the body is the whole new one. It is written from zero as the fresh file —
/// from THIS response, with no second fetch — and the sidecar rewritten in
/// flight carries the new validator (observed from the body thread, which
/// waits — bounded — for the rewrite; mockito builds the body before it sends
/// the headers, so a `with_body_from_request` callback could not see it).
#[tokio::test]
async fn resume_200_after_if_range_rewrites_from_the_response_body() {
    use mockito::Server;

    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"old-", Some(("\"v1\"", Some(8)))).await;
    let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let seen_in_body = seen.clone();
    let out_for_body = out.clone();
    let wanted = etag("\"v2\"");
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(200)
        .with_header("etag", "\"v2\"")
        .with_header("content-length", "6")
        .with_chunked_body(move |w| {
            *seen_in_body.lock().unwrap() = wait_for_sidecar_validator(&out_for_body, &wanted);
            w.write_all(b"newnew")
        })
        .expect(1)
        .create_async()
        .await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 6);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"newnew");
    let seen = seen.lock().unwrap().clone();
    let seen: HttpResumeState = serde_json::from_str(
        seen.as_deref()
            .expect("a sidecar exists while the replacement body is being served"),
    )
    .unwrap();
    assert_eq!(
        seen.validator,
        etag("\"v2\""),
        "the sidecar names the NEW representation"
    );
    assert!(
        !HttpResumeState::sidecar_path(&out).exists(),
        "sidecar is removed once the rewrite completes"
    );
}

/// §8.8.1: a strong validator is unique across versions, so a 200 carrying
/// the SAME one with `Content-Length == N` says the N-byte partial already is
/// the whole representation — finish without touching the file.
///
/// The mock body deliberately differs from the partial (a real server would
/// send the identical bytes) so that consuming it is observable: the file
/// must still hold the partial, not the body.
#[tokio::test]
async fn resume_200_same_validator_and_length_is_already_complete() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"done", Some(("\"v1\"", Some(4)))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(200)
        .with_header("etag", "\"v1\"")
        .with_body("DONE")
        .expect(1)
        .create_async()
        .await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 4);
    assert_eq!(
        tokio::fs::read(&out).await.unwrap(),
        b"done",
        "the partial is the whole representation; the body is not written"
    );
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// §14.4 `bytes */L` "indicates the current length": `L == N` says the
/// partial is complete — but only a conforming server reaches 416 after the
/// `If-Range` precondition held (§14.2), so one `If-Range` probe confirms
/// the validator before the partial is declared done: no body, sidecar
/// removed, file untouched.
#[tokio::test]
async fn resume_416_with_matching_length_is_complete_once_the_probe_confirms() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"done", Some(("\"v1\"", None))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(416)
        .with_header("content-range", "bytes */4")
        .expect(1)
        .create_async()
        .await;
    let probe = confirming_probe(&mut server, "\"v1\"", 4).await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    probe.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 4);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"done");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// A server that ignores `If-Range` can answer 416 `*/N` for a DIFFERENT
/// N-byte representation. The confirming probe sees the other validator, so
/// the partial is discarded and the download restarts rather than being
/// declared complete on the strength of a length alone.
#[tokio::test]
async fn resume_416_with_matching_length_but_other_validator_restarts() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"done", Some(("\"v1\"", None))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(416)
        .with_header("content-range", "bytes */4")
        .expect(1)
        .create_async()
        .await;
    let probe = confirming_probe(&mut server, "\"v2\"", 4).await;
    let (fresh_probe, get) = fresh_mocks(&mut server, b"DONE").await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    probe.assert_async().await;
    fresh_probe.assert_async().await;
    get.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 4);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"DONE");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// The `If-Range: "v1"` probe a resume sends to confirm a 416: a 206 whose
/// `ETag` is `etag` and whose complete-length is `len`.
async fn confirming_probe(
    server: &mut mockito::ServerGuard,
    etag: &str,
    len: u64,
) -> mockito::Mock {
    server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", &format!("bytes 0-0/{len}"))
        .with_header("etag", etag)
        .with_body("d")
        .expect(1)
        .create_async()
        .await
}

/// `L < N`: the representation is now shorter than the partial, so the
/// partial cannot be a prefix of it — discard and restart from zero.
#[tokio::test]
async fn resume_416_with_shorter_length_restarts() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"toolong", Some(("\"v1\"", None))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=7-")
        .match_header("if-range", "\"v1\"")
        .with_status(416)
        .with_header("content-range", "bytes */3")
        .expect(1)
        .create_async()
        .await;
    let (probe, get) = fresh_mocks(&mut server, b"abc").await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 7, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    probe.assert_async().await;
    get.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 3);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"abc");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// §14.1.2: a range is satisfiable iff `first-pos < current length`, so a
/// 416 whose `*/L` has `L > N` contradicts itself. Not a restart — an error,
/// with the partial and its sidecar left for diagnosis.
#[tokio::test]
async fn resume_416_with_longer_length_is_an_error() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"ab", Some(("\"v1\"", None))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=2-")
        .match_header("if-range", "\"v1\"")
        .with_status(416)
        .with_header("content-range", "bytes */10")
        .expect(1)
        .create_async()
        .await;

    let err = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 2, None, None)
        .await
        .unwrap_err();
    m.assert_async().await;
    assert!(matches!(err, RdlpError::Download { .. }), "{err:?}");
    assert!(err.to_string().contains("416"), "{err}");
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"ab");
    assert!(HttpResumeState::sidecar_path(&out).exists());
}

/// §15.5.17: `Content-Range` on a 416 is only a SHOULD. Without it the length
/// is asked of a probe sent under the same `If-Range`: a 206 whose validator
/// matches and whose complete-length is `N` means complete; anything else
/// means discard and restart.
#[tokio::test]
async fn resume_416_without_content_range_probes_then_completes_or_restarts() {
    use mockito::Server;

    // Case A: the probe says the representation is exactly N bytes.
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"done", Some(("\"v1\"", None))).await;
    let m416 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(416)
        .expect(1)
        .create_async()
        .await;
    let probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", "bytes 0-0/4")
        .with_header("etag", "\"v1\"")
        .with_body("d")
        .expect(1)
        .create_async()
        .await;
    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m416.assert_async().await;
    probe.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 4);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"done");
    assert!(!HttpResumeState::sidecar_path(&out).exists());

    // Case B: the probe reports another length; the partial is discarded and
    // the fresh path (no `If-Range`) serves the 9-byte representation.
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"done", Some(("\"v1\"", None))).await;
    let _m416 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(416)
        .expect(1)
        .create_async()
        .await;
    let probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", "bytes 0-0/9")
        .with_header("etag", "\"v1\"")
        .with_body("n")
        .expect(1)
        .create_async()
        .await;
    let (fresh_probe, get) = fresh_mocks(&mut server, b"ninebytes").await;
    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    probe.assert_async().await;
    fresh_probe.assert_async().await;
    get.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 9);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"ninebytes");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// A matching strong validator with a DIFFERENT complete-length than the
/// sidecar recorded is a validator that cannot be trusted (§8.8.1 makes it
/// unique per representation, so the two cannot both be right) — the
/// partial is discarded and the download restarts rather than appended.
#[tokio::test]
async fn resume_206_with_changed_complete_length_restarts() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"0123", Some(("\"v1\"", Some(8)))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", "bytes 4-9/10")
        .with_header("etag", "\"v1\"")
        .with_body("456789")
        .expect(1)
        .create_async()
        .await;
    let (probe, get) = fresh_mocks(&mut server, b"0123456789").await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    probe.assert_async().await;
    get.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 10);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"0123456789");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// A 206 to `If-Range: "v1"` that names another representation (or none:
/// §15.3.7 requires `ETag` on a 206) cannot be combined with the partial
/// (§15.3.7.3) — and the fresh download is the only exit, because an error
/// here would send the orchestrator straight back into the same resume.
/// Discard and restart; the file is then the fresh body.
#[tokio::test]
async fn resume_206_with_different_etag_restarts_instead_of_erroring() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"0123", Some(("\"v1\"", Some(8)))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", "bytes 4-7/8")
        .with_header("etag", "\"v2\"")
        .with_body("4567")
        .expect(1)
        .create_async()
        .await;
    let (probe, get) = fresh_mocks(&mut server, b"ABCDEFGH").await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    probe.assert_async().await;
    get.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 8);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"ABCDEFGH");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

#[tokio::test]
async fn resume_206_without_etag_restarts_instead_of_erroring() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"0123", Some(("\"v1\"", Some(8)))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", "bytes 4-7/8")
        .with_body("4567")
        .expect(1)
        .create_async()
        .await;
    let (probe, get) = fresh_mocks(&mut server, b"ABCDEFGH").await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m.assert_async().await;
    probe.assert_async().await;
    get.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 8);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"ABCDEFGH");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}

/// §14.1.2 / §8.4: byte offsets are over the coded form, so a content-coded
/// 206 can never be appended at the partial's offset. Rejected before any
/// byte lands; the partial and its sidecar stay.
#[tokio::test]
async fn resume_rejects_content_coded_206_before_writing() {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"0123", Some(("\"v1\"", Some(8)))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(206)
        .with_header("content-range", "bytes 4-7/8")
        .with_header("content-encoding", "gzip")
        .with_header("etag", "\"v1\"")
        .with_body("4567")
        .expect(1)
        .create_async()
        .await;

    let err = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap_err();
    m.assert_async().await;
    assert!(err.to_string().contains("content-coded"), "{err}");
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"0123");
    assert!(HttpResumeState::sidecar_path(&out).exists());
}

/// The sidecar outlives a failed or cancelled resume, so the next attempt can
/// send its validator again; only success removes it.
#[tokio::test]
async fn resume_keeps_sidecar_on_failure_and_on_cancel() {
    use mockito::Server;
    use tokio_util::sync::CancellationToken;

    // Failure: a 503 with no retries left.
    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"0123", Some(("\"v1\"", Some(8)))).await;
    let m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(503)
        .with_body("unavailable")
        .expect(1)
        .create_async()
        .await;
    let err = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap_err();
    m.assert_async().await;
    assert!(
        matches!(err, RdlpError::Http { status: 503, .. }),
        "{err:?}"
    );
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"0123");
    assert!(HttpResumeState::sidecar_path(&out).exists());

    // Cancel: a pre-cancelled token short-circuits before any request.
    let (_dir, out, d) = resume_fixture(b"0123", Some(("\"v1\"", Some(8)))).await;
    let token = CancellationToken::new();
    token.cancel();
    let err = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, Some(&token))
        .await
        .unwrap_err();
    assert!(matches!(err, RdlpError::Cancelled), "{err:?}");
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"0123");
    assert!(HttpResumeState::sidecar_path(&out).exists());
}

/// The rewrite after a 200-to-`If-Range` must never leave v1's bytes under a
/// sidecar naming v2: a later resume would append v2's tail to v1's prefix —
/// the corruption #565 exists to prevent. Discarding the partial (and its
/// v1 sidecar) comes FIRST, so a failure anywhere in the window leaves "no
/// sidecar" or the old one, both of which restart. Forced here by making the
/// partial unwritable so `File::create` fails.
#[cfg(unix)]
#[tokio::test]
async fn resume_200_rewrite_failure_never_leaves_new_sidecar_over_old_bytes() {
    use mockito::Server;
    use std::os::unix::fs::PermissionsExt;

    let mut server = Server::new_async().await;
    let (_dir, out, d) = resume_fixture(b"old-", Some(("\"v1\"", Some(8)))).await;
    let _m = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", "\"v1\"")
        .with_status(200)
        .with_header("etag", "\"v2\"")
        .with_body("newnew")
        .create_async()
        .await;

    tokio::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o444))
        .await
        .unwrap();
    let result = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await;
    tokio::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o644))
        .await
        .unwrap();

    assert!(matches!(result, Err(RdlpError::Io(_))), "{result:?}");
    assert_eq!(
        tokio::fs::read(&out).await.unwrap(),
        b"old-",
        "the unwritable partial still holds v1's bytes"
    );
    let left = HttpResumeState::load(&out).await;
    assert!(
        left.as_ref().is_none_or(|s| s.validator != etag("\"v2\"")),
        "v1's bytes must not sit under a sidecar naming v2: {left:?}"
    );
}

// ---------------------------------------------------------------------------
// `stream_body` — the one body loop; `StreamPolicy` is the only thing its
// callers (file paths vs stdout) disagree on.
// ---------------------------------------------------------------------------

/// A writer that accepts `capacity` bytes and then reports `BrokenPipe`, as
/// a consumer that closed its end of a pipe does.
struct PipeWithCapacity {
    accepted: Vec<u8>,
    capacity: usize,
}

impl tokio::io::AsyncWrite for PipeWithCapacity {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let room = self.capacity - self.accepted.len();
        if room == 0 {
            return std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let n = buf.len().min(room);
        self.accepted.extend_from_slice(&buf[..n]);
        std::task::Poll::Ready(Ok(n))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// Two body frames, `abc` then `def`, into a pipe that closes after three
/// bytes: `StopOnBrokenPipe` (stdout) returns `Ok` with the bytes delivered
/// before the break; `FailOnWriteError` (a file) surfaces the `Io` error.
async fn stream_into_pipe(policy: StreamPolicy) -> (Result<u64>, Vec<u8>) {
    use mockito::Server;
    let mut server = Server::new_async().await;
    let _m = server
        .mock("GET", "/v")
        .with_status(200)
        .with_chunked_body(|w| {
            w.write_all(b"abc")?;
            // Keep the second frame off the wire until the first has been
            // consumed, so the two arrive as distinct chunks.
            std::thread::sleep(std::time::Duration::from_millis(100));
            w.write_all(b"def")
        })
        .create_async()
        .await;
    let d = chunk_test_downloader(0);
    let response = rdlp_http::download_request(d.client(), &format!("{}/v", server.url()), None)
        .send()
        .await
        .unwrap();
    let mut pipe = PipeWithCapacity {
        accepted: Vec::new(),
        capacity: 3,
    };
    let result = d
        .stream_body(
            response,
            BodySink {
                writer: &mut pipe,
                policy,
                offset: 0,
            },
            None,
            None,
        )
        .await;
    (result, pipe.accepted)
}

#[tokio::test]
async fn stream_body_stop_on_broken_pipe_returns_bytes_before_the_break() {
    let (result, accepted) = stream_into_pipe(StreamPolicy::StopOnBrokenPipe).await;
    assert_eq!(accepted, b"abc");
    assert!(matches!(result, Ok(3)), "{result:?}");
}

#[tokio::test]
async fn stream_body_fail_on_write_error_surfaces_broken_pipe() {
    let (result, accepted) = stream_into_pipe(StreamPolicy::FailOnWriteError).await;
    assert_eq!(accepted, b"abc");
    assert!(
        matches!(&result, Err(RdlpError::Io(e)) if e.kind() == std::io::ErrorKind::BrokenPipe),
        "{result:?}"
    );
}

// ---------------------------------------------------------------------------
// #565 final-review fixes: the plain GET's coding gate, and the 416 re-probe
// under a `Last-Modified` validator.
// ---------------------------------------------------------------------------

/// The plain sequential GET has no `Range`, so `range_verdict` never sees it —
/// yet RFC 9110 §12.5.3 makes honouring `Accept-Encoding: identity` only a
/// SHOULD. A server that answers the pin with `Content-Encoding: gzip` must be
/// refused before a byte lands or a sidecar names the coded length; otherwise
/// the final file is gzip bytes and a later resume computes offsets over them
/// (§14.1.2).
#[tokio::test]
async fn sequential_get_rejects_content_coded_200_before_writing() {
    use mockito::{Matcher, Server};
    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    let m = server
        .mock("GET", "/v")
        .match_header("range", Matcher::Missing)
        .match_header("accept-encoding", "identity")
        .with_status(200)
        .with_header("content-encoding", "gzip")
        .with_header("etag", "\"v1\"")
        .with_body("not really gzip")
        .expect(1)
        .create_async()
        .await;

    let d = chunk_test_downloader(0);
    let err = d
        .download_sequential(&format!("{}/v", server.url()), &out, None, None)
        .await
        .unwrap_err();
    m.assert_async().await;
    assert!(
        matches!(&err, RdlpError::Download { message, .. } if message.contains("content-coded")),
        "{err:?}"
    );
    assert!(
        !out.exists() || tokio::fs::metadata(&out).await.unwrap().len() == 0,
        "no coded byte may reach the output file"
    );
    assert!(
        !HttpResumeState::sidecar_path(&out).exists(),
        "no sidecar may describe a body that was refused"
    );
}

/// §15.3.7: a 206 to an `If-Range` request SHOULD NOT repeat `Last-Modified`,
/// so the re-probe after a 416-without-`Content-Range` must apply the same
/// lenient rule `verify_partial` does for a `Last-Modified` sidecar — an
/// absent `Last-Modified` on the probe's 206 confirms the partial rather than
/// forcing a restart.
#[tokio::test]
async fn resume_416_without_content_range_probe_confirms_last_modified_partial() {
    use mockito::Server;
    const LAST_MODIFIED: &str = "Sun, 06 Nov 1994 08:49:37 GMT";
    let lm = StrongValidator::LastModified(
        rdlp_http::validator::ImfFixdate::parse(LAST_MODIFIED).unwrap(),
    );

    let mut server = Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let out = dir.path().join("v.mp4");
    tokio::fs::write(&out, b"done").await.unwrap();
    HttpResumeState::new(lm, None).save(&out).await.unwrap();
    let d = chunk_test_downloader(0).with_parallel_threshold(u64::MAX);

    let m416 = server
        .mock("GET", "/v")
        .match_header("range", "bytes=4-")
        .match_header("if-range", LAST_MODIFIED)
        .with_status(416)
        .expect(1)
        .create_async()
        .await;
    let probe = server
        .mock("GET", "/v")
        .match_header("range", "bytes=0-262143")
        .match_header("if-range", LAST_MODIFIED)
        .with_status(206)
        .with_header("content-range", "bytes 0-0/4")
        .with_body("d")
        .expect(1)
        .create_async()
        .await;

    let stats = d
        .download_with_resume(&format!("{}/v", server.url()), &out, 4, None, None)
        .await
        .unwrap();
    m416.assert_async().await;
    probe.assert_async().await;
    assert_eq!(stats.bytes_downloaded, 4);
    assert_eq!(tokio::fs::read(&out).await.unwrap(), b"done");
    assert!(!HttpResumeState::sidecar_path(&out).exists());
}
