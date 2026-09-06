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
    // ruleid: raw-url-in-log-field
    error!(url = thumbnail_url; "boom");
    // ruleid: raw-url-in-log-field
    info!(url = thumbnail_url; "info");
    // ruleid: raw-url-in-log-field
    trace!(url = thumbnail_url; "trace");
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

// Fixtures for raw-url-in-instrument-field: the tracing ATTRIBUTE form,
// not the macro-call form tested above.
// `fields(...)` can be instrument's only/first/last/middle argument, and
// independently the credential-shaped field can be fields()'s only/first/
// last/middle entry -- each combination below is a distinct regression, per
// the empirical finding that a leading or trailing "..." does not match when
// there is nothing on that side.

// ruleid: raw-url-in-instrument-field
#[instrument(fields(url = %url))]
fn sole_arg_sole_field(url: &str) {}

// ruleid: raw-url-in-instrument-field
#[instrument(skip(self), fields(url = %url))]
fn skip_then_sole_field(url: &str) {}

// ruleid: raw-url-in-instrument-field
#[instrument(fields(url = %url), skip(self))]
fn sole_field_then_skip(url: &str) {}

// ruleid: raw-url-in-instrument-field
#[instrument(skip(self), fields(url = %url, n = 1))]
fn field_first_of_many(url: &str) {}

// ruleid: raw-url-in-instrument-field
#[instrument(skip(self), fields(n = 1, url = %url))]
fn field_last_of_many(url: &str) {}

// ruleid: raw-url-in-instrument-field
#[instrument(fields(a = 1, url = %url, b = 2))]
fn field_in_middle(url: &str) {}

// ruleid: raw-url-in-instrument-field
#[instrument(skip(self), fields(host = %host))]
fn non_url_credential_field(host: &str) {}

// A dotted/qualified expression, not a bare identifier -- regression pin for
// the $X -> $...X fix (a plain generic-mode metavariable captures exactly
// ONE token and silently failed to match `format.url`; a mutation test
// against execution.rs's real fields(url = %format.url, ...) shape caught
// this uncaught before the fix).
// ruleid: raw-url-in-instrument-field
#[instrument(skip(self, downloader, format), fields(url = %format.url))]
fn dotted_field_access(format: &FormatInfo) {}

// ok: raw-url-in-instrument-field
#[instrument(skip(self), fields(url = %rdlp_redact::RedactedUrl::new(url)))]
fn sanitized_instrument_field(url: &str) {}

// The sanitized counterpart of the dotted-field case above.
// ok: raw-url-in-instrument-field
#[instrument(skip(self, downloader, format), fields(url = %rdlp_redact::RedactedUrl::new(&format.url)))]
fn sanitized_dotted_field_access(format: &FormatInfo) {}

// ok: raw-url-in-instrument-field
#[instrument(skip(self), fields(phase = %phase))]
fn non_credential_field_name(phase: &str) {}

// ok: raw-url-in-instrument-field
//
// A doc comment that QUOTES the vulnerable shape verbatim, mirroring
// extraction.rs:315, must not itself be flagged (#695).
/// `#[instrument(fields(url = %url))]` would leak the raw URL if written
/// for real -- this line only documents the pattern to avoid.
fn doc_comment_quoting_the_pattern() {}
