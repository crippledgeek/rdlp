//! Proves a URL recorded as a `tracing` field via `%` (Display) is redacted in
//! subscriber output — the mechanism the #328 instrument-span sweep relies on.
use std::io;
use std::sync::{Arc, Mutex};

use rdlp_redact::RedactedUrl;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SharedBuf {
    type Writer = SharedBuf;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn tracing_url_field_via_display_sigil_is_redacted() {
    let buf = SharedBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .without_time()
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let raw = "https://cdn/s.m4s?token=SECRET&X-Amz-Signature=DEADBEEF";
        tracing::info!(url = %RedactedUrl::new(raw), "downloading");
    });

    let out = String::from_utf8(buf.0.lock().expect("lock").clone()).expect("utf8");
    assert!(
        out.contains("token=***"),
        "field redacted in subscriber output: {out}"
    );
    assert!(
        !out.contains("SECRET"),
        "raw token must not reach the sink: {out}"
    );
    assert!(
        !out.contains("DEADBEEF"),
        "raw signature must not reach the sink: {out}"
    );
}
