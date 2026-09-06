//! The field vocabulary a boundary record is built from.
//!
//! These types exist so the convention's two structural rules are enforced by
//! the compiler rather than by review: a record names exactly one subject, and
//! a URL subject cannot carry an unredacted value. See
//! `CODING_RULES.md` "Boundary Logging" for the convention itself.
//!
//! They live here, in the leaf types crate, rather than in either boundary,
//! because both sinks need them: `rdlp-desktop`'s `AppError` constructors and
//! `rdlp-cli`'s `record_failure`. While the vocabulary lived in the desktop
//! crate the CLI hand-wrote its `action=` string, so the "a record names
//! exactly one subject" guarantee held on only one of the two boundaries the
//! convention names.

use std::fmt;

use rdlp_redact::RedactedUrl;

/// What a boundary action acted on. At most one per record.
///
/// The `Url` arm holds a [`RedactedUrl`] rather than a `&str` so a raw URL
/// cannot reach a record through this path — records persist to a file on
/// disk, so a credential in one outlives the session that wrote it.
#[derive(Clone, Copy)]
pub enum Subject<'a> {
    /// No subject worth naming.
    None,
    /// A local filesystem path. Not redacted: a path in the operator's own
    /// log is not a credential.
    Path(&'a str),
    /// A URL, redacted by its type.
    Url(RedactedUrl<'a>),
    /// A download job id.
    Job(&'a str),
}

/// What the boundary was doing when the outcome occurred.
#[derive(Clone, Copy)]
pub struct Action<'a> {
    name: &'a str,
    subject: Subject<'a>,
}

impl<'a> Action<'a> {
    /// An action with no subject.
    #[must_use]
    pub const fn new(name: &'a str) -> Self {
        Self {
            name,
            subject: Subject::None,
        }
    }

    /// An action against a specific subject.
    #[must_use]
    pub const fn with_subject(name: &'a str, subject: Subject<'a>) -> Self {
        Self { name, subject }
    }

    /// The job id this action names, if its subject is [`Subject::Job`].
    ///
    /// Lets a constructor that stores the job id in a struct field (e.g.
    /// `AppError::DownloadFailed.job_id`) read it back from the same
    /// `Action` that renders it into the record, rather than taking it as a
    /// second, driftable parameter.
    #[must_use]
    pub const fn job_id(&self) -> Option<&'a str> {
        match self.subject {
            Subject::Job(j) => Some(j),
            _ => None,
        }
    }
}

impl fmt::Display for Action<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "action={}", self.name)?;
        match &self.subject {
            Subject::None => Ok(()),
            Subject::Path(p) => write!(f, " path={p}"),
            Subject::Url(u) => write!(f, " url={u}"),
            Subject::Job(j) => write!(f, " job_id={j}"),
        }
    }
}

/// Render the one canonical terminal-record body for a failed action.
///
/// ```text
/// action=<verb> [<subject>] outcome=failed reason=<display>
/// ```
///
/// This is the single place that text is built. Before it existed the format
/// string was hand-written at eight sites across two crates (six `AppError`
/// constructors, both arms of `from_api`, `events.rs`, and the CLI's
/// `record_failure`), only one of which was pinned by an equality assertion —
/// so seven copies could drift past the `contains` assertions that guard the
/// rest.
///
/// The LEVEL is deliberately not decided here. Choosing WARN vs ERROR is a
/// property of the failing variant (`AppError::internal` is ERROR, everything
/// else is WARN), which is knowledge the desktop crate owns; this crate owns
/// only the shape.
///
/// `reason` is rendered through its `Display`, which for `AppError` and
/// `RdlpApiError` is already credential-redacting — see `CODING_RULES.md`
/// "Boundary Logging" rule 5.
#[must_use]
pub fn failure_record(action: &Action<'_>, reason: &impl fmt::Display) -> String {
    format!("{action} outcome=failed reason={reason}")
}

#[cfg(test)]
mod tests {
    use super::{Action, Subject, failure_record};
    use rdlp_redact::RedactedUrl;

    #[test]
    fn an_action_without_a_subject_renders_only_the_action() {
        assert_eq!(Action::new("reveal").to_string(), "action=reveal");
    }

    #[test]
    fn a_path_subject_is_named_and_not_redacted() {
        let a = Action::with_subject("reveal", Subject::Path("/tmp/v.mkv"));
        assert_eq!(a.to_string(), "action=reveal path=/tmp/v.mkv");
    }

    #[test]
    fn a_url_subject_is_redacted_by_its_type() {
        let raw = "https://user:hunter2@example.com/v";
        let a = Action::with_subject("analyze", Subject::Url(RedactedUrl::new(raw)));
        let shown = a.to_string();
        assert!(!shown.contains("hunter2"), "credential leaked: {shown}");
        assert!(shown.starts_with("action=analyze url="), "got: {shown}");
    }

    #[test]
    fn a_job_subject_is_named() {
        let a = Action::with_subject("download", Subject::Job("job-7"));
        assert_eq!(a.to_string(), "action=download job_id=job-7");
    }

    #[test]
    fn the_record_body_names_action_subject_outcome_and_reason() {
        let a = Action::with_subject("download", Subject::Job("job-7"));
        assert_eq!(
            failure_record(&a, &"disk full"),
            "action=download job_id=job-7 outcome=failed reason=disk full"
        );
    }
}
