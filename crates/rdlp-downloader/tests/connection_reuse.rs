//! Connection-pool reuse guard for `DownloaderRegistry::with_config_and_client`
//! (PRD 2026-06-02, item 1).
//!
//! The orchestrator builds ONE emulating `wreq::Client` per download and passes
//! it to the downloader registry so the download phase reuses the extraction
//! phase's pooled keep-alive TCP connection instead of opening a fresh
//! TCP+BoringSSL handshake. `wreq::Client` is `Arc`-internal, so the property
//! that makes this work is: a registry built via `with_config_and_client(client)`
//! shares `client`'s connection pool.
//!
//! This is unobservable through wreq's public API (no pool identity), so we
//! observe it at the wire: a tiny keep-alive HTTP/1.1 server counts accepted TCP
//! connections. We pre-warm a client (1 connection), then drive a small
//! sequential download through a registry and count how many connections the
//! server accepts.
//!
//! In the reuse path (`with_config_and_client(warm_client)`) the download rides
//! the warm client's pool, so its first request reuses the warm connection. In
//! the control path (`with_config`, which builds its own client) the registry
//! cannot see the warm pool, so every request opens a fresh connection. The
//! reuse path therefore accepts strictly fewer connections.
//!
//! Determinism note: the body is 1 MiB. The F3 probe (`GET Range: 0-262143`)
//! reads only the headers then drops the response, leaving the body unread; on
//! HTTP/1.1 hyper cannot reuse a connection with an unread body, so the probe
//! *always* consumes one connection and the actual download *always* opens one
//! more. A tiny body would let hyper sometimes auto-drain and pool the probe
//! connection, making the count flaky — 1 MiB keeps it firmly in the
//! poison-then-reopen regime while staying well under the parallel threshold
//! (so the download stays sequential / single-connection).

// Integration-test helpers below aren't `#[cfg(test)]`/`#[test]` items, so
// `allow-unwrap-in-tests` does not cover them; opt the whole file out per the
// documented convention in `clippy.toml`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rdlp_downloader::DownloaderRegistry;
use rdlp_http::HttpClientFactory;
use rdlp_types::{Config, DownloadProtocol, Format};
use tempfile::TempDir;

/// 1 MiB served for every GET — large enough that the probe's partial read
/// deterministically poisons its connection, small enough to stay sequential.
const BODY_LEN: usize = 1024 * 1024;

/// Spawn a minimal keep-alive HTTP/1.1 server on an ephemeral port that counts
/// accepted TCP connections. Returns `(base_url, connection_counter)`.
///
/// Each connection is served on its own thread in a keep-alive loop until the
/// peer goes idle (5s read timeout) or closes — so pooled-but-idle client
/// connections don't keep the connection alive past the test.
fn spawn_counting_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_for_thread = Arc::clone(&counter);
    let body = Arc::new(vec![b'a'; BODY_LEN]);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            counter_for_thread.fetch_add(1, Ordering::SeqCst);
            let body = Arc::clone(&body);
            std::thread::spawn(move || {
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let mut buf = [0u8; 8192];
                loop {
                    // Read one request (headers terminate at the first blank line).
                    let n = match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break, // peer closed or went idle
                        Ok(n) => n,
                    };
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let has_range = req.to_ascii_lowercase().contains("range:");

                    // 206 for range requests (probe + ranged GET), 200 otherwise.
                    // Either way the whole body is returned with a correct total,
                    // Accept-Ranges advertised, and the connection kept alive.
                    let total = body.len();
                    let headers = if has_range {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\n\
                             Content-Range: bytes 0-{}/{total}\r\n\
                             Accept-Ranges: bytes\r\n\
                             Content-Length: {total}\r\n\
                             Connection: keep-alive\r\n\r\n",
                            total - 1
                        )
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\n\
                             Accept-Ranges: bytes\r\n\
                             Content-Length: {total}\r\n\
                             Connection: keep-alive\r\n\r\n"
                        )
                    };
                    if stream.write_all(headers.as_bytes()).is_err()
                        || stream.write_all(&body).is_err()
                        || stream.flush().is_err()
                    {
                        break;
                    }
                }
            });
        }
    });

    (format!("http://127.0.0.1:{port}"), counter)
}

/// Drive one sequential download through `registry` for `url`.
async fn download_via(registry: &DownloaderRegistry, url: &str, out_dir: &TempDir) {
    let downloader = registry.find_downloader(url).unwrap();
    let format = Format::new("http", url, "bin", DownloadProtocol::Https);
    let output = out_dir.path().join("out.bin");
    downloader
        .download_format(&format, &output, None, None)
        .await
        .unwrap();
}

/// Pre-warm `client` by issuing one fully-consumed GET so exactly one pooled
/// keep-alive connection exists before the registry download.
async fn prewarm(client: &wreq::Client, url: &str) {
    let resp = client.get(url).send().await.unwrap();
    let _ = resp.bytes().await.unwrap();
}

#[tokio::test]
async fn reused_client_shares_connection_pool_with_caller() {
    let config = Config::default();
    let tmp = TempDir::new().unwrap();

    // --- Foundational property: a SINGLE client reuses its own pooled
    //     keep-alive connection across two sequential requests. If this ever
    //     breaks (wreq pooling regression / server quirk), the discriminator
    //     below would be meaningless, so assert it explicitly. ---
    let (foundation_url, foundation_conns) = spawn_counting_server();
    let one_client = HttpClientFactory::from_rdlp_config(&config).build();
    prewarm(&one_client, &foundation_url).await;
    prewarm(&one_client, &format!("{foundation_url}/again")).await;
    assert_eq!(
        foundation_conns.load(Ordering::SeqCst),
        1,
        "a single client must reuse its pooled connection across requests"
    );

    // --- Reuse path: registry REUSES the pre-warmed client's pool. ---
    let (reuse_url, reuse_conns) = spawn_counting_server();
    let shared_client = HttpClientFactory::from_rdlp_config(&config).build();
    prewarm(&shared_client, &reuse_url).await; // 1 connection opened + pooled
    let reuse_registry = DownloaderRegistry::with_config_and_client(&config, shared_client.clone());
    download_via(&reuse_registry, &format!("{reuse_url}/file"), &tmp).await;
    let reuse_total = reuse_conns.load(Ordering::SeqCst);

    // --- Control path: registry builds its OWN client (separate pool). ---
    let (ctrl_url, ctrl_conns) = spawn_counting_server();
    let warmed_but_unused = HttpClientFactory::from_rdlp_config(&config).build();
    prewarm(&warmed_but_unused, &ctrl_url).await; // 1 connection opened + pooled
    let ctrl_registry = DownloaderRegistry::with_config(&config);
    download_via(&ctrl_registry, &format!("{ctrl_url}/file"), &tmp).await;
    let ctrl_total = ctrl_conns.load(Ordering::SeqCst);

    // The control opened the warm connection AND its own connections for the
    // download; the reuse path rode the warm connection for the download's
    // first request, so it accepts strictly fewer.
    assert!(
        ctrl_total >= 2,
        "control (own client) must open the warm conn plus its own; got {ctrl_total}"
    );
    assert!(
        reuse_total >= 1,
        "reuse path must have opened the pre-warm connection; got {reuse_total}"
    );
    assert!(
        reuse_total < ctrl_total,
        "registry built via with_config_and_client must reuse the caller's pooled \
         connection (fewer TCP accepts) than one building its own client: \
         reuse={reuse_total} vs control={ctrl_total}"
    );
}
