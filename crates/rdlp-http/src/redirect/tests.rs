use crate::{HttpClientConfig, HttpClientFactory};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A validator that accepts everything, for tests about following rather than
/// about refusing. A `const` fn pointer rather than a `fn` item so its
/// always-`Ok` return is not a `clippy::unnecessary_wraps` finding — the
/// `Result` is required by `ssrf_guarded_redirect_policy`'s bound, not by this
/// body. (`pedantic`/`nursery` are enabled crate-wide in `lib.rs`.)
const ALLOW_ANYTHING: fn(&str) -> Result<(), std::convert::Infallible> = |_| Ok(());

/// Serve one canned response per accepted connection, forever.
fn serve(listener: TcpListener, response: &'static str, hits: Option<Arc<AtomicUsize>>) {
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            if let Some(h) = &hits {
                h.fetch_add(1, Ordering::SeqCst);
            }
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(response.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
}

/// The defect #662 describes: a public seed that redirects to a private host
/// must be refused at the hop, not followed.
///
/// The private target is a listener we control that answers 200, rather than a
/// real link-local address. A redirect to 169.254.169.254 would also produce
/// `Err` today — by connect timeout — so the test would pass against the
/// unpatched client for the wrong reason.
#[tokio::test(flavor = "current_thread")]
async fn a_redirect_to_a_private_host_is_refused() {
    let private = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let private_addr = private.local_addr().unwrap();
    serve(
        private,
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecret",
        None,
    );

    let seed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let seed_addr = seed.local_addr().unwrap();
    let redirect: &'static str = Box::leak(
        format!("HTTP/1.1 302 Found\r\nLocation: http://{private_addr}/latest/meta-data/\r\nContent-Length: 0\r\n\r\n")
            .into_boxed_str(),
    );
    serve(seed, redirect, None);

    let result = HttpClientFactory::default()
        .build()
        .get(format!("http://{seed_addr}/m.m3u8"))
        .send()
        .await;

    match result {
        Err(e) => {
            // Pin the cause. Asserting only that the body did not leak would
            // be satisfied by any error at all — a bind failure, a reset, a
            // future timeout regression — because the refusal never contains
            // the body in the first place.
            assert!(
                e.is_redirect(),
                "must fail at the redirect policy, not in transport: {e}"
            );
            let msg = e.to_string();
            assert!(
                msg.contains("refused by SSRF validation"),
                "must be the SSRF refusal, not another redirect error: {msg}"
            );
            assert!(!msg.contains("secret"), "must not echo the target: {msg}");
        }
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            panic!("redirect to a private host was FOLLOWED: status {status}, body {body:?}");
        }
    }
}

/// The guarded policy must still FOLLOW.
///
/// Loopback is refused by the production validator by design, so the following
/// behaviour is driven through an injected permissive one.
#[tokio::test(flavor = "current_thread")]
async fn the_guarded_policy_still_follows_302_redirects() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let req = String::from_utf8_lossy(&buf).to_string();
            let response = if req.contains("GET /redir") {
                "HTTP/1.1 302 Found\r\nLocation: /dest\r\nContent-Length: 0\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nfollowed!"
            };
            let _ = sock.write_all(response.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });

    let resp = HttpClientFactory::default()
        .build_with_validator(None, ALLOW_ANYTHING)
        .get(format!("http://{addr}/redir"))
        .send()
        .await
        .expect("request must succeed");
    assert_eq!(resp.status().as_u16(), 200, "redirect was not followed");
    assert_eq!(resp.text().await.unwrap(), "followed!");
}

/// What the policy is handed for a RELATIVE `Location`.
///
/// Load-bearing, and not answerable from the wreq source without following it
/// some way: the policy validates a string, and `validate_url_security`
/// rejects anything it cannot parse as an absolute URL. If wreq passed `/dest`
/// through unresolved, every relative redirect on the internet would be
/// refused as malformed. It does not — it resolves against the previous hop —
/// and this test is what says so.
#[tokio::test(flavor = "current_thread")]
async fn a_relative_location_reaches_the_policy_already_resolved() {
    use std::sync::Mutex;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let req = String::from_utf8_lossy(&buf).to_string();
            let response = if req.contains("GET /redir") {
                "HTTP/1.1 302 Found\r\nLocation: /dest\r\nContent-Length: 0\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
            };
            let _ = sock.write_all(response.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });

    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = {
        let seen = Arc::clone(&seen);
        move |url: &str| -> Result<(), std::convert::Infallible> {
            seen.lock().unwrap().push(url.to_string());
            Ok(())
        }
    };

    let client = HttpClientFactory::default().build_with_validator(None, recorder);
    let _ = client.get(format!("http://{addr}/redir")).send().await;

    // Cloned out so the guard drops before the assertions — a held
    // `MutexGuard` across a panicking assert is `significant_drop_tightening`.
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "policy should see exactly one hop: {seen:?}");
    assert_eq!(
        seen.first().map(String::as_str),
        Some(format!("http://{addr}/dest").as_str()),
        "a relative Location must reach the policy resolved against the \
         previous hop, or the validator would reject it as unparseable"
    );
}

/// `Policy::custom` does not apply a hop limit — wreq documents that the
/// caller must handle it — so the guarded policy delegates to `limited`.
///
/// Driven through `HttpClientConfig::with_max_redirects` rather than by
/// passing the cap to the policy directly, so this also proves `build_inner`
/// reads the config field. Without that, replacing `self.config.max_redirects`
/// with a literal leaves the suite green.
///
/// The assertion counts the requests the server sees. An earlier version
/// asserted only `expect_err`, and passed with the cap removed: the endless
/// chain ran until the 60s read timeout and returned an error that satisfied
/// it. A test for a bound has to count.
#[tokio::test(flavor = "current_thread")]
async fn the_hop_limit_comes_from_the_config() {
    const MAX_HOPS: usize = 3;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    serve(
        listener,
        "HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n",
        Some(Arc::clone(&hits)),
    );

    let config = HttpClientConfig::default().with_max_redirects(MAX_HOPS);
    let err = HttpClientFactory::from_config(&config)
        .build_with_validator(None, ALLOW_ANYTHING)
        .get(format!("http://{addr}/loop"))
        .send()
        .await
        .expect_err("an endless redirect loop must fail, not hang");

    assert!(err.is_redirect(), "must fail at the redirect layer: {err}");
    assert!(
        !err.to_string().contains("refused by SSRF validation"),
        "must fail on the hop cap, not the SSRF gate: {err}"
    );
    // `limited` compares `previous.len() > max`, so `max` follows are allowed:
    // the initial request plus MAX_HOPS, and the next hop is refused.
    assert_eq!(
        hits.load(Ordering::SeqCst),
        MAX_HOPS + 1,
        "the chain must stop at the configured cap, not run to a timeout"
    );
}

/// The refusal names the refused target, redacted.
///
/// Both halves matter and each is asserted: the target must be there, because
/// "which hop failed" is the whole diagnostic value, and the userinfo must not,
/// because the string reaches logs through `wreq::Error`'s Display.
///
/// An earlier version asserted only the absence and was a tautology —
/// `SecurityError::PrivateHost` carries the host alone, so no credential was
/// ever in the message and removing the redaction left the test green.
/// Including the target is what gives the redaction something to do.
#[tokio::test(flavor = "current_thread")]
async fn a_refused_redirect_names_its_target_without_userinfo() {
    let seed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = seed.local_addr().unwrap();
    serve(
        seed,
        "HTTP/1.1 302 Found\r\nLocation: http://admin:hunter2@169.254.169.254/latest/meta-data/\r\nContent-Length: 0\r\n\r\n",
        None,
    );

    let err = HttpClientFactory::default()
        .build()
        .get(format!("http://{addr}/seed"))
        .send()
        .await
        .expect_err("a redirect to a link-local host must be refused");

    // Two renderings, deliberately: `Display` is what an operator reads in a
    // log, and the derived `Debug` prints every field whatever `Display` does
    // — so asserting presence against Debug would hold even if the target
    // never reached the message.
    let displayed = err.to_string();
    let rendered = format!("{err} {err:?}");

    // Asserted on the PATH, not the host: the validator's own error text
    // already names the host, so a host assertion passes even with the target
    // dropped. The path exists only in the target.
    assert!(
        displayed.contains("/latest/meta-data/"),
        "the refusal must name the target it refused: {displayed}"
    );
    assert!(
        !rendered.contains("hunter2"),
        "the refusal leaked the password: {rendered}"
    );
    assert!(
        !rendered.contains("admin"),
        "the refusal leaked the username: {rendered}"
    );
}
