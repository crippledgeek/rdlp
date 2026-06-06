//! Resume state for native-HLS pre-resolved-fragment downloads.
//!
//! Mirrors `dash::state::DashDownloadState`: persisted to
//! `<output>.hls_state.json`, matched on schema version + a path-only
//! fingerprint so CDN host/token rotation does not break resume. Load/save
//! are async (`tokio::fs`) because the workspace bans blocking `std::fs` in
//! async contexts; the file is tiny.

// Callers land in Task 4 (resume integration in fragments.rs); this attribute
// is removed there. Until then the pub items are reachable only from tests.
#![allow(dead_code)]

use std::path::Path;
use std::time::SystemTime;

use rdlp_types::Fragment;
use serde::{Deserialize, Serialize};
use tokio::fs;

/// Current schema version. Bump on incompatible field changes.
pub const STATE_VERSION: u32 = 1;

/// Persisted state of an in-progress native-HLS fragment download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlsResumeState {
    /// Schema version. Mismatches force a fresh start.
    pub state_version: u32,
    /// FNV-1a-64 of the ordered, path-only fragment URLs (+ count). CDN-tolerant:
    /// host and query are ignored so token/host rotation does not break resume.
    pub fingerprint: u64,
    /// Total fragments in the resolved list.
    pub total_fragments: u64,
    /// Number of fragments fully written to the output so far.
    pub fragments_done: u64,
    /// Byte length of the output confirmed flushed at the last checkpoint.
    pub byte_len: u64,
    /// Unix epoch seconds — for stale-state diagnosis.
    pub updated_at: u64,
}

/// Ordered, path-only FNV-1a-64 over the fragment list. Stable across runs;
/// ignores host/query so CDN token/host rotation does not break resume. The
/// fragment count is folded in first so two lists with the same paths but
/// different lengths never collide.
#[must_use]
pub fn fragment_fingerprint(fragments: &[Fragment]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(PRIME);
        }
    };
    feed(&(fragments.len() as u64).to_le_bytes());
    for f in fragments {
        let path = url::Url::parse(&f.url).map_or_else(|_| f.url.clone(), |u| u.path().to_string());
        feed(path.as_bytes());
        feed(b"\n");
    }
    h
}

impl HlsResumeState {
    /// Fresh state for a new download of `total_fragments` fragments.
    #[must_use]
    pub fn new(fingerprint: u64, total_fragments: u64) -> Self {
        Self {
            state_version: STATE_VERSION,
            fingerprint,
            total_fragments,
            fragments_done: 0,
            byte_len: 0,
            updated_at: now_secs(),
        }
    }

    /// Load iff present, parseable, and matching version + fingerprint + total.
    /// `None` ⇒ the caller starts fresh (fail-safe).
    #[must_use]
    pub async fn load_matching(
        path: &Path,
        fingerprint: u64,
        total_fragments: u64,
    ) -> Option<Self> {
        let body = fs::read_to_string(path).await.ok()?;
        let s: Self = serde_json::from_str(&body).ok()?;
        (s.state_version == STATE_VERSION
            && s.fingerprint == fingerprint
            && s.total_fragments == total_fragments)
            .then_some(s)
    }

    /// Persist state to `path` atomically, updating the `updated_at` stamp.
    ///
    /// # Errors
    /// Returns the underlying I/O error from the atomic write, or a JSON
    /// serialization error wrapped as `io::Error::other`.
    pub async fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.updated_at = now_secs();
        crate::atomic::atomic_write_json(path, self.clone()).await
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::Fragment;

    fn frag(url: &str) -> Fragment {
        Fragment {
            url: url.to_string(),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: None,
            filesize: None,
        }
    }

    #[test]
    fn fingerprint_is_path_only_stable_across_host_and_query() {
        let a = vec![
            frag("https://cdn1.example.com/v/seg-0.ts?token=AAA"),
            frag("https://cdn1.example.com/v/seg-1.ts?token=AAA"),
        ];
        let b = vec![
            frag("https://cdn2.OTHER.net/v/seg-0.ts?token=ZZZ"),
            frag("https://cdn2.OTHER.net/v/seg-1.ts?token=ZZZ"),
        ];
        assert_eq!(
            fragment_fingerprint(&a),
            fragment_fingerprint(&b),
            "host + query must not affect the fingerprint (path-only)"
        );
    }

    #[test]
    fn fingerprint_changes_on_path_change() {
        let a = vec![frag("https://x/seg-0.ts"), frag("https://x/seg-1.ts")];
        let b = vec![frag("https://x/seg-0.ts"), frag("https://x/seg-2.ts")];
        assert_ne!(fragment_fingerprint(&a), fragment_fingerprint(&b));
    }

    #[test]
    fn fingerprint_changes_on_count_change() {
        let a = vec![frag("https://x/seg-0.ts")];
        let b = vec![frag("https://x/seg-0.ts"), frag("https://x/seg-0.ts")];
        assert_ne!(
            fragment_fingerprint(&a),
            fragment_fingerprint(&b),
            "count is folded into the fingerprint"
        );
    }

    #[tokio::test]
    async fn save_then_load_matching_roundtrips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        let fp = 0xDEAD_BEEF;
        let mut s = HlsResumeState::new(fp, 10);
        s.fragments_done = 4;
        s.byte_len = 4096;
        s.save(&path).await.expect("save");

        let loaded = HlsResumeState::load_matching(&path, fp, 10)
            .await
            .expect("must load matching");
        assert_eq!(loaded.fragments_done, 4);
        assert_eq!(loaded.byte_len, 4096);
        assert_eq!(loaded.total_fragments, 10);
    }

    #[test]
    fn fingerprint_handles_relative_urls_via_fallback() {
        // Relative paths don't parse as absolute URLs → fingerprint falls back
        // to the raw string. Must be deterministic and not panic, and must
        // still distinguish different relative paths.
        let a = vec![frag("seg-0.ts"), frag("seg-1.ts")];
        let b = vec![frag("seg-0.ts"), frag("seg-1.ts")];
        let c = vec![frag("seg-0.ts"), frag("seg-2.ts")];
        assert_eq!(
            fragment_fingerprint(&a),
            fragment_fingerprint(&b),
            "identical relative-path lists must hash equally (deterministic fallback)"
        );
        assert_ne!(
            fragment_fingerprint(&a),
            fragment_fingerprint(&c),
            "differing relative paths must still diverge"
        );
    }

    #[tokio::test]
    async fn load_matching_none_on_fingerprint_total_version_or_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        let mut s = HlsResumeState::new(111, 10);
        s.fragments_done = 4;
        s.byte_len = 4096;
        s.save(&path).await.expect("save");

        assert!(
            HlsResumeState::load_matching(&path, 222, 10)
                .await
                .is_none()
        );
        assert!(HlsResumeState::load_matching(&path, 111, 9).await.is_none());
        let missing = dir.path().join("nope.hls_state.json");
        assert!(
            HlsResumeState::load_matching(&missing, 111, 10)
                .await
                .is_none()
        );

        let bogus = r#"{"state_version":999,"fingerprint":111,"total_fragments":10,"fragments_done":4,"byte_len":4096,"updated_at":0}"#.to_string();
        tokio::fs::write(&path, bogus).await.expect("write bogus");
        assert!(
            HlsResumeState::load_matching(&path, 111, 10)
                .await
                .is_none()
        );
    }
}
