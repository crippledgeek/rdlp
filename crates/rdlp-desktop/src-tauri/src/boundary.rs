//! The field vocabulary a boundary record is built from.
//!
//! These types exist so the convention's two structural rules are enforced by
//! the compiler rather than by review: a record names exactly one subject, and
//! a URL subject cannot carry an unredacted value. See
//! `CODING_RULES.md` "Boundary Logging" for the convention itself.

use std::fmt;

use rdlp_redact::RedactedUrl;

/// What a boundary action acted on. At most one per record.
///
/// The `Url` arm holds a [`RedactedUrl`] rather than a `&str` so a raw URL
/// cannot reach a record through this path — records persist to a file on
/// disk, so a credential in one outlives the session that wrote it.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired to real call sites in Task 2 of #695; self-cleaning once used outside tests"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired to real call sites in Task 2 of #695; self-cleaning once used outside tests"
    )
)]
#[derive(Clone, Copy)]
pub struct Action<'a> {
    name: &'a str,
    subject: Subject<'a>,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired to real call sites in Task 2 of #695; self-cleaning once used outside tests"
    )
)]
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

#[cfg(test)]
mod tests {
    use super::{Action, Subject};
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
}
