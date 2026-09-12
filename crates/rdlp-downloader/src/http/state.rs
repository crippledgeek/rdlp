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
use crate::atomic::{atomic_write_json, now_secs};

/// Current schema version. Bump on incompatible field changes.
pub(crate) const STATE_VERSION: u32 = 1;
const SIDECAR_SUFFIX: &str = ".http_state.json";

/// Persisted state of an in-progress plain-HTTP download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HttpResumeState {
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
    pub(crate) fn sidecar_path(output: &Path) -> PathBuf {
        let mut name = output
            .file_name()
            .map(std::ffi::OsStr::to_os_string)
            .unwrap_or_default();
        name.push(SIDECAR_SUFFIX);
        output.with_file_name(name)
    }

    /// `None` when the sidecar is missing, unparsable, or of another schema
    /// version — every one of which means "start over" (fail-safe).
    pub(crate) async fn load(output: &Path) -> Option<Self> {
        let body = tokio::fs::read_to_string(Self::sidecar_path(output))
            .await
            .ok()?;
        let s: Self = serde_json::from_str(&body).ok()?;
        (s.state_version == STATE_VERSION).then_some(s)
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
    pub(crate) async fn remove(output: &Path) {
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
}

impl Source {
    pub(crate) fn new(url: &str, validator: Option<StrongValidator>) -> Self {
        Self {
            url: url.to_owned(),
            validator,
        }
    }

    /// A source with no validator. Test-only: every production path now
    /// threads the probe's or the sidecar's validator through [`Self::new`],
    /// and a chunk test that wants "none was offered" says so here.
    #[cfg(test)]
    pub(crate) fn unverified(url: &str) -> Self {
        Self::new(url, None)
    }

    /// What a ranged request for `range` against this source asked for, so
    /// the answer can be checked against it.
    pub(crate) const fn meta(&self, range: RangeSpec) -> RangedRequestMeta<'_> {
        RangedRequestMeta {
            range,
            sent_validator: self.validator.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_http::validator::StrongEntityTag;

    fn etag(s: &str) -> StrongValidator {
        StrongValidator::ETag(StrongEntityTag::parse(s).unwrap())
    }

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

    /// `meta` is what every chunk's verdict is judged against, so the
    /// validator it carries must be exactly the source's — `unverified`
    /// sends none, `new` with one sends that one.
    #[test]
    fn meta_carries_the_source_validator() {
        let span = RangeSpec::span(0, 9).unwrap();
        let none = Source::unverified("http://h/v");
        assert!(none.meta(span).sent_validator.is_none());
        assert_eq!(none.meta(span).range, span);

        let v = etag("\"a\"");
        let some = Source::new("http://h/v", Some(v.clone()));
        assert_eq!(some.meta(span).sent_validator, Some(&v));
        assert_eq!(some.url, "http://h/v");
    }
}
