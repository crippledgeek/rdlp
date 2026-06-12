// Semgrep --test fixture for the raw-url-in-log-field rule.
// Lines annotated with "ruleid:" MUST be flagged; lines annotated with "ok:" MUST NOT.
// This file lives outside crates/ and is never compiled.
fn f(thumbnail_url: &str, e: u8, fmt: &str) {
    // ruleid: raw-url-in-log-field
    debug!(url = thumbnail_url; "Downloading thumbnail");
    // ruleid: raw-url-in-log-field
    warn!(url = thumbnail_url; "rejected: {e}");
    // ruleid: raw-url-in-log-field
    warn!(url = thumbnail_url, status = 503; "non-success");
    // ok: raw-url-in-log-field
    warn!(url = RedactedUrl::new(thumbnail_url); "rejected: {e}");
    // ok: raw-url-in-log-field
    warn!(url = RedactedUrl::new(thumbnail_url), status = 503; "non-success");
    // ok: raw-url-in-log-field
    debug!(url = rdlp_redact::RedactedUrl::new(thumbnail_url); "ok");
    // ok: raw-url-in-log-field
    let url = thumbnail_url;
    // ok: raw-url-in-log-field
    warn!(url = sanitize_for_logging(thumbnail_url); "ok");
    // ok: raw-url-in-log-field
    debug!(url = rdlp_security::sanitize_for_logging(&fmt).as_str(), n = 1; "ok");
}
