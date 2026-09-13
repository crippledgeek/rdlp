//! Resume state for plain-HTTP downloads: the strong validator the download
//! started under, so a later `Range` request can carry `If-Range` and a
//! later 206 can be checked against it (RFC 9110 §13.1.5, §15.3.7.3).
//!
//! Mirrors `fragments::state::HlsResumeState` and `dash::state::DashDownloadState`:
//! persisted next to the output as `<output>.http_state.json` with
//! `atomic_write_json`, owned by the downloader, invisible to the
//! orchestrator. A state is written only when a validator exists — "no
//! sidecar" *means* "no validator", and the resume path then restarts
//! (spec: RFC 9110 §15.3.7.3 grants combining only under a shared strong
//! validator). `updated_at` is diagnostic; §8.8.1 makes the validator valid
//! for the resource's lifetime, so nothing here expires.

use std::path::{Path, PathBuf};

use log::warn;
use rdlp_http::{RangeSpec, StrongValidator};
use serde::{Deserialize, Serialize};

use super::verdict::RangedRequestMeta;
use crate::atomic::{atomic_write_json, now_secs, read_json_sidecar};

/// Current schema version. Bump on incompatible field changes.
pub(crate) const STATE_VERSION: u32 = 1;
const SIDECAR_SUFFIX: &str = ".http_state.json";

/// Persisted state of an in-progress plain-HTTP download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResumeState {
    /// Schema version. Mismatches are treated as "no state".
    pub state_version: u32,
    /// The strong validator of the representation whose bytes are on disk.
    pub validator: StrongValidator,
    /// `Content-Range` complete-length (206) or `Content-Length` (200) of the
    /// response that produced the bytes on disk, when known.
    pub complete_length: Option<u64>,
    /// Unix epoch seconds — for stale-state diagnosis only.
    pub updated_at: u64,
}

impl HttpResumeState {
    pub(crate) fn new(validator: StrongValidator, complete_length: Option<u64>) -> Self {
        Self {
            state_version: STATE_VERSION,
            validator,
            complete_length,
            updated_at: now_secs(),
        }
    }

    /// `<output file name>.http_state.json`, in the output's directory.
    #[must_use]
    pub fn sidecar_path(output: &Path) -> PathBuf {
        let mut name = output
            .file_name()
            .map(std::ffi::OsStr::to_os_string)
            .unwrap_or_default();
        name.push(SIDECAR_SUFFIX);
        output.with_file_name(name)
    }

    /// `None` when the sidecar is missing, unparsable, over the size bound,
    /// or of another schema version — every one of which means "start over"
    /// (fail-safe).
    pub(crate) async fn load(output: &Path) -> Option<Self> {
        let s: Self = read_json_sidecar(&Self::sidecar_path(output)).await?;
        (s.state_version == STATE_VERSION).then_some(s)
    }

    /// Make the sidecar say exactly what `validator` says: write it when the
    /// response offered a strong validator, remove any sidecar when it did
    /// not. "No sidecar" must mean "no validator" — a sidecar left by an
    /// earlier response would describe bytes this one is about to replace —
    /// so the two outcomes are one operation, shared by the fresh probe, the
    /// sequential GET and the resume's 200-rewrite.
    ///
    /// # Errors
    /// The write's I/O error; a download that cannot record what it fetched
    /// cannot later be resumed safely, so this is surfaced, not downgraded.
    pub(crate) async fn record(
        output: &Path,
        validator: Option<StrongValidator>,
        complete_length: Option<u64>,
    ) -> std::io::Result<()> {
        if let Some(v) = validator {
            Self::new(v, complete_length).save(output).await
        } else {
            Self::remove(output).await;
            Ok(())
        }
    }

    /// Persist atomically next to `output`, refreshing `updated_at`.
    ///
    /// # Errors
    /// The underlying I/O error from the atomic write, or a JSON
    /// serialization error wrapped as `io::Error::other`.
    pub(crate) async fn save(&mut self, output: &Path) -> std::io::Result<()> {
        self.updated_at = now_secs();
        atomic_write_json(&Self::sidecar_path(output), self.clone()).await
    }

    /// Delete the sidecar. A missing file is the desired end state; any other
    /// failure is logged rather than surfaced, because a stale sidecar can
    /// only cost a re-probe on the next run, never data.
    ///
    /// Public because the sidecar outlives the downloader's own success path
    /// in one case: the orchestrator finds a `.rdlp-part` already complete
    /// (`resolve_resume`'s `Complete` outcome) and finalizes it without ever
    /// calling the downloader, so it removes the sidecar through this — the
    /// sidecar owner's API — rather than by naming the file itself.
    pub async fn remove(output: &Path) {
        let path = Self::sidecar_path(output);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!(path:? = path; "Failed to remove resume sidecar: {e}"),
        }
    }
}

/// The representation a download is fetching: its URL and, when the server
/// offered one, the strong validator every ranged request carries as
/// `If-Range` and every 206 is checked against.
///
/// One value rather than a URL plus a separately threaded validator, so the
/// two cannot drift apart between the probe and the chunk fan-out.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    pub url: String,
    pub validator: Option<StrongValidator>,
    /// The representation's complete length as recorded alongside
    /// `validator` — the sidecar's on a resume, the probe's on a fresh
    /// download — when known. Under §8.8.1 a strong validator names one
    /// length, so an open-ended 206 that says `*` is still held to it
    /// ([`RangedRequestMeta::known_total`]).
    pub complete_length: Option<u64>,
}

impl Source {
    pub(crate) fn new(url: &str, validator: Option<StrongValidator>) -> Self {
        Self {
            url: url.to_owned(),
            validator,
            complete_length: None,
        }
    }

    /// The length recorded under this source's validator, when known.
    #[must_use]
    pub(crate) const fn with_complete_length(mut self, complete_length: Option<u64>) -> Self {
        self.complete_length = complete_length;
        self
    }

    /// What a ranged request for `range` against this source asked for, so
    /// the answer can be checked against it.
    pub(crate) const fn meta(&self, range: RangeSpec) -> RangedRequestMeta<'_> {
        RangedRequestMeta {
            range,
            sent_validator: self.validator.as_ref(),
            known_total: self.complete_length,
        }
    }
}

/// A strong-`ETag` validator from its raw quoted form, for the tests of
/// every module in `http` that builds one (`state`, `verdict`, `tests`).
#[cfg(test)]
pub(crate) fn etag(s: &str) -> StrongValidator {
    StrongValidator::ETag(rdlp_http::validator::StrongEntityTag::parse(s).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sidecar_round_trips_and_sits_next_to_the_output() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("video.rdlp-part.mp4");
        let v = etag("\"v1\"");
        let mut s = HttpResumeState::new(v.clone(), Some(1234));
        s.save(&out).await.unwrap();
        assert_eq!(
            HttpResumeState::sidecar_path(&out),
            dir.path().join("video.rdlp-part.mp4.http_state.json")
        );
        let back = HttpResumeState::load(&out).await.unwrap();
        assert_eq!(back.validator, v);
        assert_eq!(back.complete_length, Some(1234));
    }

    #[tokio::test]
    async fn sidecar_version_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("v.mp4");
        let mut s = HttpResumeState::new(etag("\"v1\""), None);
        s.state_version = STATE_VERSION + 1;
        s.save(&out).await.unwrap();
        assert!(HttpResumeState::load(&out).await.is_none());
    }

    #[tokio::test]
    async fn sidecar_missing_or_garbage_is_none_and_remove_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("v.mp4");
        assert!(HttpResumeState::load(&out).await.is_none());
        tokio::fs::write(HttpResumeState::sidecar_path(&out), b"{not json")
            .await
            .unwrap();
        assert!(HttpResumeState::load(&out).await.is_none());
        HttpResumeState::remove(&out).await;
        HttpResumeState::remove(&out).await;
        assert!(!HttpResumeState::sidecar_path(&out).exists());
    }

    /// A sidecar over the size bound is not read, however well-formed: the
    /// bound is what keeps a corrupt or hostile file from being pulled into
    /// memory whole, so it must bite even when the JSON inside is valid.
    #[tokio::test]
    async fn oversized_sidecar_is_not_loaded_even_when_valid() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("v.mp4");
        let mut body = serde_json::to_string(&HttpResumeState::new(etag("\"v1\""), None)).unwrap();
        // JSON tolerates trailing whitespace, so the padded file still parses.
        let pad = usize::try_from(crate::atomic::MAX_SIDECAR_BYTES).unwrap() + 1 - body.len();
        body.extend(std::iter::repeat_n(' ', pad));
        tokio::fs::write(HttpResumeState::sidecar_path(&out), &body)
            .await
            .unwrap();
        assert!(HttpResumeState::load(&out).await.is_none());
    }

    /// `meta` is what every chunk's verdict is judged against, so the
    /// validator it carries must be exactly the source's — `None` sends
    /// none, `Some` sends that one.
    #[test]
    fn meta_carries_the_source_validator() {
        let span = RangeSpec::span(0, 9).unwrap();
        let none = Source::new("http://h/v", None);
        assert!(none.meta(span).sent_validator.is_none());
        assert_eq!(none.meta(span).range, span);

        let v = etag("\"a\"");
        let some = Source::new("http://h/v", Some(v.clone()));
        assert_eq!(some.meta(span).sent_validator, Some(&v));
        assert_eq!(some.url, "http://h/v");
    }
}
