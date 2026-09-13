//! Resume state for native-HLS pre-resolved-fragment downloads.
//!
//! Mirrors `dash::state::DashDownloadState`: persisted to
//! `<output>.hls_state.json`, matched on schema version + a content
//! fingerprint + anchor validator, see `fragment_fingerprint`. Load/save
//! are async (`tokio::fs`) because the workspace bans blocking `std::fs` in
//! async contexts; the load is bounded by `atomic::MAX_SIDECAR_BYTES`.
//!
//! The sidecar also carries a running CRC-32 of the output so a resume can
//! prove the partial's content, not just its length (#676): a crash can leave
//! the output at `byte_len` bytes with an unwritten (zeroed) tail on
//! filesystems that commit size before data (see `crate::atomic`). Nothing is
//! fsynced; the mismatch is detected on resume and the download starts fresh.

use std::path::Path;

use rdlp_types::Fragment;
use serde::{Deserialize, Serialize};

use crate::atomic::{now_secs, read_json_sidecar};
use crate::fingerprint::Fnv1a64;

/// Current schema version. Bump on incompatible field changes.
/// v2 (#676) added `stream_crc32`; v3 (#746) widened the fingerprint to
/// manifest content and added `anchor_validator`. Older sidecars are
/// rejected ⇒ fresh start.
pub const STATE_VERSION: u32 = 3;

/// Persisted state of an in-progress native-HLS fragment download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlsResumeState {
    /// Schema version. Mismatches force a fresh start.
    pub state_version: u32,
    /// FNV-1a-64 over the fragment list's content-bearing fields; see
    /// `fragment_fingerprint`. CDN-tolerant: host and query are ignored so
    /// token/host rotation does not break resume.
    pub fingerprint: u64,
    /// Total fragments in the resolved list.
    pub total_fragments: u64,
    /// Number of fragments fully written to the output so far.
    pub fragments_done: u64,
    /// Byte length of the output confirmed flushed at the last checkpoint.
    pub byte_len: u64,
    /// CRC-32/IEEE of `output[0..byte_len]` (`crate::atomic::crc32_of_prefix`).
    /// Re-computed on resume; a mismatch means the partial's content is not what was written (a
    /// non-durable tail after a crash) and the download starts fresh.
    pub stream_crc32: u32,
    /// The strong validator (RFC 9110 §8.8) fragment 0's response offered
    /// when this download started, `None` when the origin offered none. On
    /// resume it is sent back as `If-Range` against the *current* list's
    /// fragment-0 URL (`crate::revalidate`): a 206/200 that still names it
    /// proves the representation is the one the partial was written from.
    pub anchor_validator: Option<rdlp_http::StrongValidator>,
    /// Unix epoch seconds — for stale-state diagnosis.
    pub updated_at: u64,
}

/// Ordered FNV-1a-64 over what the extractor resolved from the media
/// playlist: the fragment count, then per fragment its URL *path*, byte
/// range (`#EXT-X-BYTERANGE`), init-segment path and range (`#EXT-X-MAP`),
/// and duration bits (`#EXTINF`). Host and query are ignored so CDN token /
/// host rotation does not break resume.
///
/// Which signals, and why (#746). The downloader never sees the playlist
/// text — only `Fragment`s — so the fingerprint uses every content-bearing
/// field `Fragment` carries. Durations and byte ranges move on a re-encode
/// or an ad-insertion variant; init path changes on a repackage.
/// Rejected: `#EXT-X-MEDIA-SEQUENCE` and `#EXT-X-DISCONTINUITY-SEQUENCE`
/// are not in `Fragment` and are meaningless for a `VoD` playlist that starts
/// at 0; `filesize` is "rarely populated" (`rdlp_types::Fragment`) and
/// would make the fingerprint depend on whether an extractor happened to
/// fill it. A same-durations, same-ranges re-encode is undetectable from
/// the manifest and is what `HlsResumeState::anchor_validator` is for.
#[must_use]
pub fn fragment_fingerprint(fragments: &[Fragment]) -> u64 {
    let mut h = Fnv1a64::new();
    h.feed_u64(fragments.len() as u64);
    for f in fragments {
        h.feed_url_path(&f.url);
        h.feed_opt_range(f.byte_range);
        match &f.init_url {
            Some(u) => {
                h.feed(&[1]);
                h.feed_url_path(u);
            }
            None => h.feed(&[0]),
        }
        h.feed_opt_range(f.init_byte_range);
        h.feed_opt_f64_bits(f.duration);
    }
    h.finish()
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
            stream_crc32: 0,
            anchor_validator: None,
            updated_at: now_secs(),
        }
    }

    /// Load iff present, parseable, and matching version + fingerprint + total.
    /// `None` ⇒ the caller starts fresh (fail-safe).
    ///
    /// Also rejects internally-inconsistent state: `fragments_done > 0` with
    /// `byte_len == 0` is physically impossible from normal operation
    /// (`byte_len` always advances with `fragments_done`). A corrupted/zeroed
    /// sidecar in that shape would otherwise pass the gate, seek the output to
    /// offset 0, and skip the already-done fragments — silently dropping their
    /// bytes.
    #[must_use]
    pub async fn load_matching(
        path: &Path,
        fingerprint: u64,
        total_fragments: u64,
    ) -> Option<Self> {
        let s: Self = read_json_sidecar(path).await?;
        (s.state_version == STATE_VERSION
            && s.fingerprint == fingerprint
            && s.total_fragments == total_fragments
            && !(s.fragments_done > 0 && s.byte_len == 0))
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

        let bogus = r#"{"state_version":999,"fingerprint":111,"total_fragments":10,"fragments_done":4,"byte_len":4096,"stream_crc32":0,"updated_at":0}"#.to_string();
        tokio::fs::write(&path, bogus).await.expect("write bogus");
        assert!(
            HlsResumeState::load_matching(&path, 111, 10)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn load_matching_none_on_inconsistent_done_without_bytes() {
        // fragments_done > 0 with byte_len == 0 is physically impossible from
        // normal operation; a corrupted sidecar like this must be rejected so
        // resume can't seek to 0 and skip real fragments (silent corruption).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        // Otherwise-valid v3 shape so the inconsistency gate is what rejects it.
        let bogus = r#"{"state_version":3,"fingerprint":111,"total_fragments":10,"fragments_done":4,"byte_len":0,"stream_crc32":0,"anchor_validator":null,"updated_at":0}"#.to_string();
        tokio::fs::write(&path, bogus).await.expect("write bogus");
        assert!(
            HlsResumeState::load_matching(&path, 111, 10)
                .await
                .is_none(),
            "inconsistent done>0/byte_len==0 sidecar must be rejected"
        );
    }

    #[tokio::test]
    async fn load_matching_none_on_v1_sidecar_without_crc() {
        // A real pre-#676 sidecar: version 1, no `stream_crc32`. Must be
        // rejected so the upgrade starts fresh instead of trusting an
        // unverifiable partial.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        let v1 = r#"{"state_version":1,"fingerprint":111,"total_fragments":10,"fragments_done":4,"byte_len":4096,"updated_at":0}"#;
        tokio::fs::write(&path, v1).await.expect("write v1");
        assert!(
            HlsResumeState::load_matching(&path, 111, 10)
                .await
                .is_none(),
            "v1 sidecar must not load"
        );
    }

    #[tokio::test]
    async fn load_matching_rejects_version_1_even_when_crc_field_is_present() {
        // Isolates the version gate from serde's missing-field rejection: a
        // document that parses as the current struct but claims version 1
        // must still be refused.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        let v1 = r#"{"state_version":1,"fingerprint":111,"total_fragments":10,"fragments_done":4,"byte_len":4096,"stream_crc32":0,"updated_at":0}"#;
        tokio::fs::write(&path, v1).await.expect("write v1");
        assert!(
            HlsResumeState::load_matching(&path, 111, 10)
                .await
                .is_none(),
            "version gate must reject 1 regardless of field shape"
        );
    }

    #[tokio::test]
    async fn stream_crc32_roundtrips_through_save_and_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        let mut s = HlsResumeState::new(7, 3);
        s.fragments_done = 1;
        s.byte_len = 9;
        s.stream_crc32 = 0xCBF4_3926;
        s.save(&path).await.expect("save");
        let loaded = HlsResumeState::load_matching(&path, 7, 3)
            .await
            .expect("must load");
        assert_eq!(loaded.stream_crc32, 0xCBF4_3926);
    }

    #[test]
    fn fingerprint_changes_when_durations_change_but_paths_do_not() {
        let mut a = vec![frag("https://x/seg-0.ts"), frag("https://x/seg-1.ts")];
        let mut b = a.clone();
        a[0].duration = Some(4.0);
        a[1].duration = Some(4.0);
        b[0].duration = Some(4.0);
        b[1].duration = Some(6.006);
        assert_ne!(
            fragment_fingerprint(&a),
            fragment_fingerprint(&b),
            "#EXTINF is a content signal (#746)"
        );
    }

    #[test]
    fn fingerprint_changes_when_byte_ranges_move() {
        let mut a = vec![frag("https://x/all.ts"), frag("https://x/all.ts")];
        let mut b = a.clone();
        a[0].byte_range = Some((0, 100));
        a[1].byte_range = Some((100, 250));
        b[0].byte_range = Some((0, 100));
        b[1].byte_range = Some((100, 260));
        assert_ne!(
            fragment_fingerprint(&a),
            fragment_fingerprint(&b),
            "#EXT-X-BYTERANGE moves on a re-encode"
        );
    }

    #[test]
    fn fingerprint_changes_when_init_segment_path_changes_but_is_host_tolerant() {
        let mut a = vec![frag("https://x/seg-0.m4s")];
        let mut b = a.clone();
        let mut c = a.clone();
        a[0].init_url = Some("https://cdn1/v/init.mp4?t=1".into());
        b[0].init_url = Some("https://cdn2/v/init.mp4?t=2".into());
        c[0].init_url = Some("https://cdn1/v/init-v2.mp4".into());
        assert_eq!(fragment_fingerprint(&a), fragment_fingerprint(&b));
        assert_ne!(fragment_fingerprint(&a), fragment_fingerprint(&c));
    }

    #[tokio::test]
    async fn load_matching_rejects_v2_sidecar_without_anchor_field() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        let v2 = r#"{"state_version":2,"fingerprint":111,"total_fragments":10,"fragments_done":4,"byte_len":4096,"stream_crc32":0,"updated_at":0}"#;
        tokio::fs::write(&path, v2).await.expect("write v2");
        assert!(
            HlsResumeState::load_matching(&path, 111, 10)
                .await
                .is_none(),
            "a v2 sidecar has no anchor and a narrower fingerprint — fresh start"
        );
    }

    #[tokio::test]
    async fn anchor_validator_roundtrips_through_save_and_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.hls_state.json");
        let mut s = HlsResumeState::new(7, 3);
        s.fragments_done = 1;
        s.byte_len = 9;
        s.anchor_validator =
            Some(rdlp_http::StrongValidator::try_from("etag:\"e1\"".to_string()).expect("strong"));
        s.save(&path).await.expect("save");
        let loaded = HlsResumeState::load_matching(&path, 7, 3)
            .await
            .expect("load");
        assert_eq!(loaded.anchor_validator, s.anchor_validator);
    }
}
