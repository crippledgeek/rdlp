//! Resume state for DASH downloads.
//!
//! State is persisted to `<output>.dash_state.json` and loaded on retry,
//! matched on schema version, MPD path, representation ids, and a
//! manifest-content fingerprint (`manifest_fingerprint`, #746); the anchor
//! validator is revalidated separately (`crate::revalidate`). URL matching
//! within the fingerprint stays path-only so a CDN host swap doesn't break
//! resume.
//!
//! Load/save are async (`tokio::fs`) because the workspace clippy config
//! bans blocking `std::fs` in async contexts. The load is bounded by
//! `atomic::MAX_SIDECAR_BYTES`, shared by every resume sidecar.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::atomic::{now_secs, read_json_sidecar};
use crate::dash::manifest::ParsedManifest;
use crate::dash::segments::SegmentPlan;
use crate::fingerprint::Fnv1a64;

/// Current schema version.
///
/// v2 (#677) added per-part expected byte lengths (`init_video_len` /
/// `init_audio_len` / the lengths in `completed_segments`), so a resumed
/// part is trusted only when its on-disk length matches exactly. v3 (#746)
/// adds `manifest_fingerprint` and `anchor_validator`; a v2 sidecar matched
/// on MPD path + representation ids only, so a regenerated MPD with the
/// same segment names resumed onto different content. Rejected ⇒ fresh
/// start.
pub const STATE_VERSION: u32 = 3;

/// Zero-based index of a media segment within one representation.
pub type SegmentIndex = u64;
/// Byte length of a part as written to disk.
pub type ByteLen = u64;

/// What identifies the download a sidecar belongs to. Grouped so `new` and
/// `load_matching` stay under three parameters and cannot swap two `&str`s
/// (`limit-function-arguments.md`).
pub struct DashIdentity<'a> {
    /// The MPD URL a resumed download must still be pointed at.
    pub mpd_url: &'a Url,
    /// `id` of the chosen video representation.
    pub video_repr_id: &'a str,
    /// `id` of the chosen audio representation, when one is present.
    pub audio_repr_id: Option<&'a str>,
    /// [`manifest_fingerprint`] of the currently-resolved MPD.
    pub manifest_fingerprint: u64,
}

/// FNV-1a-64 over the parsed MPD's content-bearing fields.
///
/// Feeds the chosen representation ids, the period duration,
/// `MPD@publishTime` when present, and the durable segment plan (template
/// string, start number, timescale, segment duration and count, timeline
/// `<S>` entries, or the `SegmentList`'s URL paths), plus the init
/// URL/range. `BaseURL` hosts are not fed — the plan's strings are relative
/// templates and list URLs are fed path-only — so a CDN host swap keeps
/// resuming.
///
/// Rejected signals: `Representation@bandwidth`/`@codecs`/`@mimeType` change
/// with a re-encode but not with an ad-insertion repackage that keeps the
/// encode; they add nothing the plan does not already separate. A same-plan,
/// same-duration re-encode is undetectable here and is what
/// `DashDownloadState::anchor_validator` catches.
#[must_use]
pub fn manifest_fingerprint(parsed: &ParsedManifest) -> u64 {
    let mut h = Fnv1a64::new();
    h.feed_opt_str(Some(&parsed.video.id));
    h.feed_opt_str(parsed.audio.as_ref().map(|a| a.id.as_str()));
    h.feed_u64(parsed.period_duration.as_secs());
    h.feed_u64(u64::from(parsed.period_duration.subsec_nanos()));
    h.feed_opt_str(parsed.publish_time.as_deref());
    feed_plan(&mut h, &parsed.video.plan);
    if let Some(a) = &parsed.audio {
        feed_plan(&mut h, &a.plan);
    }
    h.finish()
}

fn feed_plan(h: &mut Fnv1a64, plan: &SegmentPlan) {
    match plan {
        SegmentPlan::Template(t) => {
            h.feed(&[1]);
            h.feed_opt_url_path(t.init.as_deref());
            h.feed_opt_range(t.init_byte_range);
            h.feed_url_path(&t.media);
            h.feed_u64(t.start_number);
            h.feed_u64(t.timescale);
            h.feed_u64(t.segment_duration_ts);
            h.feed_u64(t.total_segments);
        }
        SegmentPlan::Timeline(t) => {
            h.feed(&[2]);
            h.feed_opt_url_path(t.init.as_deref());
            h.feed_opt_range(t.init_byte_range);
            h.feed_url_path(&t.media);
            h.feed_u64(t.start_number);
            h.feed_u64(t.timescale);
            h.feed_u64(t.entries.len() as u64);
            for e in &t.entries {
                h.feed_opt_u64(e.t);
                h.feed_u64(e.d);
                h.feed_opt_i64(e.r);
            }
        }
        SegmentPlan::List(l) => {
            h.feed(&[3]);
            h.feed_opt_url_path(l.init.as_deref());
            h.feed_opt_range(l.init_byte_range);
            h.feed_u64(l.urls.len() as u64);
            for u in &l.urls {
                h.feed_url_path(u);
            }
        }
    }
}

/// Persisted state of an in-progress DASH download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashDownloadState {
    /// Schema version. Mismatches force a fresh start.
    pub state_version: u32,
    /// Path component of the MPD URL (path-only — CDN-tolerant).
    pub mpd_path: String,
    /// `id` of the chosen video representation.
    pub video_repr_id: String,
    /// `id` of the chosen audio representation, when one is present.
    pub audio_repr_id: Option<String>,
    /// [`manifest_fingerprint`] of the MPD this download was started from.
    /// A regenerated MPD with the same path and representation ids but a
    /// different segment plan produces a different value here, so resume is
    /// refused rather than continuing onto different content (#746).
    pub manifest_fingerprint: u64,
    /// Byte length of the video init segment once written to disk, `None`
    /// until then. Resume trusts the on-disk part only when its length
    /// matches this value exactly.
    pub init_video_len: Option<ByteLen>,
    /// Byte length of the audio init segment once written to disk, `None`
    /// until then. Same matching rule as `init_video_len`.
    pub init_audio_len: Option<ByteLen>,
    /// Strong validator (RFC 9110 §8.8) of the video init segment — or of
    /// the first video segment when the plan has no init — as offered when
    /// it was first fetched. Revalidated with `If-Range` on resume
    /// (`crate::revalidate`). Written only through [`Self::record_anchor`].
    pub anchor_validator: Option<rdlp_http::StrongValidator>,
    /// `repr_id -> (segment index -> expected byte length)` for completed
    /// segments. `BTreeMap` so the serialised sidecar is deterministic.
    pub completed_segments: HashMap<String, BTreeMap<SegmentIndex, ByteLen>>,
    /// Unix epoch seconds — for stale-state diagnosis.
    pub updated_at: u64,
}

impl DashDownloadState {
    /// Construct a fresh state for the given identity.
    #[must_use]
    pub fn new(identity: &DashIdentity<'_>) -> Self {
        Self {
            state_version: STATE_VERSION,
            mpd_path: identity.mpd_url.path().to_string(),
            video_repr_id: identity.video_repr_id.to_string(),
            audio_repr_id: identity.audio_repr_id.map(str::to_string),
            manifest_fingerprint: identity.manifest_fingerprint,
            init_video_len: None,
            init_audio_len: None,
            anchor_validator: None,
            completed_segments: HashMap::new(),
            updated_at: now_secs(),
        }
    }

    /// Load state if present and matching the requested identity (path-only
    /// URL match, representation ids, and manifest fingerprint).
    /// Returns `None` for: missing file, parse failure, an over-bound file,
    /// version mismatch, path mismatch, repr-id mismatch, or a fingerprint
    /// that no longer matches the resolved MPD (#746).
    #[must_use]
    pub async fn load_matching(path: &Path, identity: &DashIdentity<'_>) -> Option<Self> {
        let s: Self = read_json_sidecar(path).await?;
        if s.state_version != STATE_VERSION
            || s.mpd_path != identity.mpd_url.path()
            || s.video_repr_id != identity.video_repr_id
            || s.audio_repr_id.as_deref() != identity.audio_repr_id
            || s.manifest_fingerprint != identity.manifest_fingerprint
        {
            return None;
        }
        Some(s)
    }

    /// Persist state to `path`, updating the `updated_at` stamp.
    ///
    /// # Errors
    /// Returns the underlying I/O error from the file write, or a JSON
    /// serialization error wrapped as `io::Error::other`.
    pub async fn save(&mut self, path: &Path) -> std::io::Result<()> {
        self.updated_at = now_secs();
        crate::atomic::atomic_write_json(path, self.clone()).await
    }

    /// Record the validator the anchor part was fetched with — unless one is
    /// already recorded, in which case `offered` is ignored.
    ///
    /// The anchor identifies the representation the parts on disk were
    /// written from, and only a fresh start (no sidecar) or a resume whose
    /// probe just `Confirmed` that anchor reaches a fetch of the anchor
    /// part. A re-fetch (a torn part) from the same representation therefore
    /// yields the same validator, or none from an edge that omits it:
    /// replacing a `Some` with `None` would silently strip every later
    /// resume of its revalidation, and replacing it with a different `Some`
    /// would hide a representation change that happened between the probe
    /// and the fetch — keeping the first recorded anchor lets the next
    /// resume's probe detect that change instead.
    pub fn record_anchor(&mut self, offered: Option<rdlp_http::StrongValidator>) {
        if self.anchor_validator.is_none() {
            self.anchor_validator = offered;
        }
    }

    /// Record segment `idx` of representation `repr_id` as completed with
    /// its written byte length `len`. Overwrites any prior record for the
    /// same index (idempotent for retries).
    pub fn record_segment(&mut self, repr_id: &str, idx: SegmentIndex, len: ByteLen) {
        self.completed_segments
            .entry(repr_id.to_string())
            .or_default()
            .insert(idx, len);
    }

    /// Returns the recorded byte length for segment `idx` of `repr_id`, or
    /// `None` if it has not been recorded.
    #[must_use]
    pub fn recorded_len(&self, repr_id: &str, idx: SegmentIndex) -> Option<ByteLen> {
        self.completed_segments.get(repr_id)?.get(&idx).copied()
    }

    /// Drop the record for segment `idx` of `repr_id`, so the caller
    /// re-fetches it. No-op if the segment was never recorded.
    pub fn forget_segment(&mut self, repr_id: &str, idx: SegmentIndex) {
        if let Some(v) = self.completed_segments.get_mut(repr_id) {
            v.remove(&idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dash::manifest;

    fn parsed(mpd: &str) -> ParsedManifest {
        manifest::parse_mpd(
            mpd,
            &Url::parse("https://cdn.example.com/p/manifest.mpd").unwrap(),
        )
        .expect("parse")
    }

    const MPD_A: &str = r#"<?xml version="1.0"?><MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT12S" minBufferTime="PT2S"><Period duration="PT12S"><AdaptationSet contentType="video"><Representation id="v1" bandwidth="500000" mimeType="video/mp4"><SegmentTemplate timescale="1000" duration="6000" startNumber="1" initialization="vinit.mp4" media="vseg-$Number$.m4s"/></Representation></AdaptationSet></Period></MPD>"#;

    #[test]
    fn manifest_fingerprint_changes_when_segment_duration_changes() {
        let b = MPD_A.replace(r#"duration="6000""#, r#"duration="4000""#);
        assert_ne!(
            manifest_fingerprint(&parsed(MPD_A)),
            manifest_fingerprint(&parsed(&b))
        );
    }

    #[test]
    fn manifest_fingerprint_changes_with_publish_time() {
        let b = MPD_A.replace(
            r#"type="static""#,
            r#"type="static" publishTime="2026-09-13T00:00:00Z""#,
        );
        assert_ne!(
            manifest_fingerprint(&parsed(MPD_A)),
            manifest_fingerprint(&parsed(&b))
        );
    }

    #[test]
    fn manifest_fingerprint_is_stable_across_base_url_hosts() {
        let a = MPD_A.replace(
            "<Period",
            "<BaseURL>https://cdn1.example/x/</BaseURL><Period",
        );
        let b = MPD_A.replace("<Period", "<BaseURL>https://cdn2.other/x/</BaseURL><Period");
        assert_eq!(
            manifest_fingerprint(&parsed(&a)),
            manifest_fingerprint(&parsed(&b))
        );
    }

    #[test]
    fn manifest_fingerprint_is_stable_across_absolute_template_urls_on_different_hosts() {
        // `SegmentTemplate@initialization`/`@media` MAY themselves be
        // absolute URLs (not just relative templates resolved against
        // BaseURL) — a CDN host swap there must not perturb the fingerprint
        // either, same as the BaseURL case above (#746 round-1 finding 4).
        let a = MPD_A.replace(
            r#"initialization="vinit.mp4" media="vseg-$Number$.m4s""#,
            r#"initialization="https://cdn1.example/vinit.mp4" media="https://cdn1.example/vseg-$Number$.m4s""#,
        );
        let b = MPD_A.replace(
            r#"initialization="vinit.mp4" media="vseg-$Number$.m4s""#,
            r#"initialization="https://cdn2.other/vinit.mp4" media="https://cdn2.other/vseg-$Number$.m4s""#,
        );
        assert_eq!(
            manifest_fingerprint(&parsed(&a)),
            manifest_fingerprint(&parsed(&b)),
            "absolute init/media URLs differing only by host must hash equally"
        );
    }

    #[tokio::test]
    async fn load_matching_rejects_fingerprint_mismatch_and_v2_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").unwrap();
        let id_a = DashIdentity {
            mpd_url: &url,
            video_repr_id: "v1",
            audio_repr_id: None,
            manifest_fingerprint: 1,
        };
        let id_b = DashIdentity {
            manifest_fingerprint: 2,
            ..id_a
        };
        let mut s = DashDownloadState::new(&id_a);
        s.record_segment("v1", 0, 5);
        s.save(&path).await.unwrap();
        assert!(
            DashDownloadState::load_matching(&path, &id_a)
                .await
                .is_some()
        );
        assert!(
            DashDownloadState::load_matching(&path, &id_b)
                .await
                .is_none(),
            "same paths, changed manifest ⇒ rejected"
        );
        let v2 = format!(
            r#"{{"state_version":2,"mpd_path":"{}","video_repr_id":"v1","audio_repr_id":null,"init_video_len":null,"init_audio_len":null,"completed_segments":{{"v1":{{"0":5}}}},"updated_at":0}}"#,
            url.path()
        );
        tokio::fs::write(&path, v2).await.unwrap();
        assert!(
            DashDownloadState::load_matching(&path, &id_a)
                .await
                .is_none(),
            "v2 has no fingerprint/anchor"
        );
    }

    #[tokio::test]
    async fn save_then_load_matching_roundtrips_lengths_through_atomic_writer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let identity = DashIdentity {
            mpd_url: &url,
            video_repr_id: "v1",
            audio_repr_id: Some("a1"),
            manifest_fingerprint: 42,
        };
        let mut state = DashDownloadState::new(&identity);
        state.record_segment("v1", 0, 100);
        state.record_segment("v1", 1, 200);
        state.init_video_len = Some(50);
        state.save(&path).await.expect("save");

        let loaded = DashDownloadState::load_matching(&path, &identity)
            .await
            .expect("must load matching state");
        assert_eq!(loaded.state_version, STATE_VERSION);
        assert_eq!(loaded.video_repr_id, "v1");
        assert_eq!(loaded.init_video_len, Some(50));
        assert_eq!(loaded.recorded_len("v1", 0), Some(100));
        assert_eq!(loaded.recorded_len("v1", 1), Some(200));
    }

    #[tokio::test]
    async fn v1_shaped_sidecar_is_rejected_not_misread() {
        // A v1 sidecar has no lengths to validate a resumed part against —
        // it must be rejected outright rather than partially parsed, per
        // the STATE_VERSION doc-comment (#677).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let v1_json = format!(
            r#"{{"state_version":1,"mpd_path":"{}","video_repr_id":"v1","audio_repr_id":null,"init_video_done":true,"init_audio_done":false,"completed_segments":{{"v1":[0]}},"updated_at":0}}"#,
            url.path()
        );
        tokio::fs::write(&path, v1_json)
            .await
            .expect("write v1 sidecar");

        let identity = DashIdentity {
            mpd_url: &url,
            video_repr_id: "v1",
            audio_repr_id: None,
            manifest_fingerprint: 0,
        };
        assert!(
            DashDownloadState::load_matching(&path, &identity)
                .await
                .is_none(),
            "a v1-shaped sidecar must not load — it carries no lengths"
        );
    }

    fn identity_v1(url: &Url) -> DashIdentity<'_> {
        DashIdentity {
            mpd_url: url,
            video_repr_id: "v1",
            audio_repr_id: None,
            manifest_fingerprint: 0,
        }
    }
    fn etag(s: &str) -> rdlp_http::StrongValidator {
        rdlp_http::StrongValidator::try_from(format!("etag:{s}")).expect("strong etag")
    }

    #[test]
    fn record_anchor_sets_when_none_is_recorded() {
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&identity_v1(&url));
        state.record_anchor(Some(etag("\"a\"")));
        assert_eq!(state.anchor_validator, Some(etag("\"a\"")));
    }

    #[test]
    fn record_anchor_never_downgrades_some_to_none() {
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&identity_v1(&url));
        state.record_anchor(Some(etag("\"a\"")));
        state.record_anchor(None);
        assert_eq!(
            state.anchor_validator,
            Some(etag("\"a\"")),
            "an edge that omits the validator must not erase the recorded anchor"
        );
    }

    #[test]
    fn record_anchor_keeps_the_first_recorded_validator() {
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&identity_v1(&url));
        state.record_anchor(Some(etag("\"a\"")));
        state.record_anchor(Some(etag("\"b\"")));
        assert_eq!(
            state.anchor_validator,
            Some(etag("\"a\"")),
            "the parts were written from the first anchor; a later one must not mask a change"
        );
    }

    #[test]
    fn record_anchor_none_onto_none_stays_none() {
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let mut state = DashDownloadState::new(&identity_v1(&url));
        state.record_anchor(None);
        assert_eq!(state.anchor_validator, None);
    }

    #[test]
    fn forget_segment_clears_recorded_len() {
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let identity = DashIdentity {
            mpd_url: &url,
            video_repr_id: "v1",
            audio_repr_id: None,
            manifest_fingerprint: 0,
        };
        let mut state = DashDownloadState::new(&identity);
        state.record_segment("v1", 0, 100);
        assert_eq!(state.recorded_len("v1", 0), Some(100));

        state.forget_segment("v1", 0);
        assert_eq!(state.recorded_len("v1", 0), None);
    }

    #[tokio::test]
    async fn v2_shaped_sidecar_with_old_version_number_is_rejected() {
        // Pins the version gate ONLY: a body that parses cleanly into the
        // CURRENT struct shape (built via `new`, so every other field
        // matches `identity`) but claims an older `state_version` must
        // still be rejected. Built through `new`/`save` (not raw JSON) so
        // only the `state_version` comparison in `load_matching` can be
        // what rejects it — raw v2-shaped JSON (missing `manifest_fingerprint`
        // entirely) is covered by
        // `load_matching_rejects_fingerprint_mismatch_and_v2_sidecar` and
        // would be rejected by serde before this clause ever ran.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("v.dash_state.json");
        let url = Url::parse("https://cdn.example.com/path/manifest.mpd").expect("url");
        let identity = DashIdentity {
            mpd_url: &url,
            video_repr_id: "v1",
            audio_repr_id: None,
            manifest_fingerprint: 0,
        };
        let mut state = DashDownloadState::new(&identity);
        state.record_segment("v1", 0, 2);
        state.state_version = STATE_VERSION - 1;
        state.save(&path).await.expect("save");

        assert!(
            DashDownloadState::load_matching(&path, &identity)
                .await
                .is_none(),
            "an older state_version must be rejected even when every other field matches"
        );
    }
}
