//! The materialised network/download settings — one owner for their defaults.

use const_format::formatcp;
use serde::Serialize;

/// The resolved network and download settings a runtime consumer reads.
///
/// rdlp layers these ten values three deep, and every layer but the last
/// keeps them optional:
///
/// 1. **Base** — [`Config`](crate::Config), loaded from `config.toml` (or
///    [`Config::default()`](crate::Config::default)). Its network fields are
///    `Option<u64>`: `None` means "not set here".
/// 2. **Overlay** — a frontend's per-request or per-profile settings
///    (rdlp-api's `NetworkOptions`, the desktop's `AppSettings`), merged
///    over the base. `None` there means *inherit* from the layer below, and
///    is never materialised into a stored default (#610).
/// 3. **Effective** — this struct. [`Config::effective_network`] collapses
///    the `Option`s once, substituting [`EffectiveNetwork::DEFAULT`] for any
///    field still unset, and hands consumers concrete values.
///
/// `DEFAULT` is the *only* place the ten default values live: rdlp-http,
/// rdlp-downloader, rdlp-extractor and rdlp-api read them from here rather than
/// carrying a copy, so a value cannot drift between the crate that documents
/// a default and the one that applies it (#611). This struct is also the
/// payload the desktop GUI's settings placeholders read — served over IPC
/// inside [`NetworkDefaults`] by a `network_defaults` command — so the GUI
/// need not hold a fifth copy either.
///
/// [`Config::effective_network`]: crate::Config::effective_network
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EffectiveNetwork {
    /// Connect-axis timeout in seconds (TCP + TLS handshake).
    ///
    /// The `Config` field keeps its historical name `socket_timeout`; the
    /// semantics are connect-only.
    pub socket_timeout_secs: u64,

    /// Read-axis timeout in seconds: per-read inactivity, not a total.
    pub read_timeout_secs: u64,

    /// Idle keep-alive connection eviction timeout in seconds. `0` is the
    /// sentinel for "disable idle eviction entirely"; rdlp-http maps it to
    /// `pool_idle_timeout(None)` at the wreq boundary.
    ///
    /// The default sits intentionally **below** common server keep-alive
    /// timeouts (nginx's default `keepalive_timeout` is 75 s; many CDN edges
    /// use 60 s) so the pool evicts an idle connection *before* the server
    /// silently closes it. Otherwise hyper hands back a dead socket on reuse
    /// and returns "connection closed before message completed"
    /// (`IncompleteMessage`), forcing an avoidable retry.
    ///
    /// This is the common-case trigger only, not the sole defence: hyper-util
    /// auto-retries once when a pooled connection is found closed before any
    /// bytes are written, and rdlp's `with_retry` / `download_chunk_with_retry`
    /// layers cover residual mid-transfer truncation — those MUST stay in
    /// place. Reference: reqwest and Go both default to 90 s; lowering below
    /// the server's keep-alive is the nginx-documented client-side mitigation
    /// (PRD 2026-06-02 item 3 / research §F-E). rdlp-http carries a
    /// compile-time assertion that the default stays below 75.
    pub pool_idle_timeout_secs: u64,

    /// Total download timeout in seconds: the whole download of one
    /// file/format must complete within this.
    pub download_timeout_secs: u64,

    /// Merge (mux/concat) timeout in seconds: the chunk/segment merge must
    /// complete within this.
    pub merge_timeout_secs: u64,

    /// Number of fragments/segments fetched concurrently.
    ///
    /// The default is a power of two well below the H2
    /// `SETTINGS_MAX_CONCURRENT_STREAMS` (RFC default 100) and conservative
    /// enough for an H1.1 fallback; see
    /// [`Config::concurrent_fragments`](crate::Config::concurrent_fragments)
    /// for the memory bound it participates in.
    pub concurrent_fragments: usize,

    /// I/O buffer size in bytes for download writes.
    pub buffer_size: usize,

    /// Minimum file size in bytes at which the HTTP downloader switches from
    /// sequential to parallel chunked download. Below this, fan-out overhead
    /// (HEAD probes, the chunk-merge step) outweighs the throughput gain.
    pub parallel_threshold: u64,

    /// Wall-clock cap in seconds on the single HEAD probe (with Range-GET
    /// fallback) used to detect content-length on non-HLS formats.
    pub hls_head_probe_timeout_secs: u64,

    /// Wall-clock budget in seconds for the orchestrator's HLS-expansion pass
    /// over one extractor result: every HLS row that arrives without
    /// pre-resolved fragments is expanded within this budget, and rows still
    /// unexpanded when it runs out are dropped with one warning.
    ///
    /// The default equals the largest per-call budget a plugin itself gets
    /// (`rdlp_plugin::adapter::SEARCH_TIMEOUT`, the `plugin_timeout_search_s`
    /// default): this pass is host work a plugin's rows trigger after its own
    /// call has returned, so it is held to the same ceiling rather than left
    /// open-ended. Comfortably above a healthy ladder — each row is one
    /// playlist round trip, and the rows are capped at the WIT boundary
    /// (`rdlp_plugin::convert::MAX_PLUGIN_FORMATS`).
    pub hls_expansion_timeout_secs: u64,
}

impl EffectiveNetwork {
    /// The ten network/download defaults. The single owner — no other crate
    /// holds a literal copy of these values.
    pub const DEFAULT: Self = Self {
        socket_timeout_secs: 30,
        read_timeout_secs: 60,
        pool_idle_timeout_secs: 60,
        download_timeout_secs: 3600,
        merge_timeout_secs: 1800,
        concurrent_fragments: 8,
        buffer_size: 2 * 1024 * 1024,
        parallel_threshold: 10 * 1024 * 1024,
        hls_head_probe_timeout_secs: 5,
        hls_expansion_timeout_secs: 60,
    };

    /// The allowed range of each of the ten fields. The single owner of the
    /// bounds: `Config::validate`, the desktop's `AppSettings::validate_security`
    /// (both through [`NetworkFields::first_out_of_range`]) and the Settings
    /// UI's numeric controls (over IPC, in [`NetworkDefaults`]) all read these
    /// rather than restating a table (#611 review).
    pub const RANGES: NetworkRanges = NetworkRanges {
        socket_timeout_secs: NetworkRange { min: 1, max: 300 },
        read_timeout_secs: NetworkRange { min: 1, max: 600 },
        pool_idle_timeout_secs: NetworkRange { min: 0, max: 3600 },
        download_timeout_secs: NetworkRange { min: 1, max: 86400 },
        merge_timeout_secs: NetworkRange { min: 1, max: 86400 },
        concurrent_fragments: NetworkRange { min: 1, max: 64 },
        buffer_size: NetworkRange {
            min: 1,
            max: MAX_BYTE_SETTING,
        },
        parallel_threshold: NetworkRange {
            min: 1,
            max: MAX_BYTE_SETTING,
        },
        hls_head_probe_timeout_secs: NetworkRange { min: 1, max: 300 },
        hls_expansion_timeout_secs: NetworkRange { min: 1, max: 600 },
    };
}

/// Ceiling for the two byte-valued fields (`buffer_size`,
/// `parallel_threshold`): 1 GiB. Above it a single buffer or a "small file"
/// threshold is a memory hazard, not a tuning.
const MAX_BYTE_SETTING: u64 = 1024 * 1024 * 1024;

/// An inclusive bound on one network/download field, in the field's own unit
/// (seconds, a count, or bytes).
///
/// Serialized to the desktop as `{min, max}` so the Settings UI's numeric
/// controls clamp to the same bounds the validators enforce, without a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NetworkRange {
    /// Inclusive lower bound.
    pub min: u64,
    /// Inclusive upper bound.
    pub max: u64,
}

impl NetworkRange {
    /// `true` when `value` is within `min..=max`.
    #[must_use]
    pub const fn contains(self, value: u64) -> bool {
        self.min <= value && value <= self.max
    }
}

/// The allowed range of every [`EffectiveNetwork`] field, one [`NetworkRange`]
/// per field under the same name. See [`EffectiveNetwork::RANGES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NetworkRanges {
    /// Bounds of [`EffectiveNetwork::socket_timeout_secs`].
    pub socket_timeout_secs: NetworkRange,
    /// Bounds of [`EffectiveNetwork::read_timeout_secs`].
    pub read_timeout_secs: NetworkRange,
    /// Bounds of [`EffectiveNetwork::pool_idle_timeout_secs`]; `min` is the
    /// `0` "eviction disabled" sentinel.
    pub pool_idle_timeout_secs: NetworkRange,
    /// Bounds of [`EffectiveNetwork::download_timeout_secs`].
    pub download_timeout_secs: NetworkRange,
    /// Bounds of [`EffectiveNetwork::merge_timeout_secs`].
    pub merge_timeout_secs: NetworkRange,
    /// Bounds of [`EffectiveNetwork::concurrent_fragments`] (caps peak
    /// transient memory under parallel fragment fetch).
    pub concurrent_fragments: NetworkRange,
    /// Bounds of [`EffectiveNetwork::buffer_size`], in bytes.
    pub buffer_size: NetworkRange,
    /// Bounds of [`EffectiveNetwork::parallel_threshold`], in bytes.
    pub parallel_threshold: NetworkRange,
    /// Bounds of [`EffectiveNetwork::hls_head_probe_timeout_secs`].
    pub hls_head_probe_timeout_secs: NetworkRange,
    /// Bounds of [`EffectiveNetwork::hls_expansion_timeout_secs`].
    pub hls_expansion_timeout_secs: NetworkRange,
}

/// The network payload the desktop's Settings view reads in ONE IPC call.
///
/// Served by the `network_defaults` command from `Config::network_defaults`:
/// what an empty field inherits, what the GUI seeds when the inherited value
/// cannot express the user's intent (re-enabling idle eviction over an
/// inherited `0`), and the bounds its controls clamp to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NetworkDefaults {
    /// The base configuration resolved through `Config::effective_network`.
    pub effective: EffectiveNetwork,
    /// [`EffectiveNetwork::DEFAULT`], before any `config.toml` layering.
    pub builtin: EffectiveNetwork,
    /// [`EffectiveNetwork::RANGES`].
    pub ranges: NetworkRanges,
}

// The `OutOfRange` reasons, formatted at compile time from `RANGES` so each
// cites the owner's bounds rather than restating them. Module consts: a
// `formatcp!` in a method body expands to an `unsafe` block.
const R: NetworkRanges = EffectiveNetwork::RANGES;
const SOCKET_TIMEOUT_REASON: &str = formatcp!(
    "must be {}..={} seconds",
    R.socket_timeout_secs.min,
    R.socket_timeout_secs.max
);
const READ_TIMEOUT_REASON: &str = formatcp!(
    "must be {}..={} seconds",
    R.read_timeout_secs.min,
    R.read_timeout_secs.max
);
const POOL_IDLE_TIMEOUT_REASON: &str = formatcp!(
    "must be {}..={} seconds (0 = disabled)",
    R.pool_idle_timeout_secs.min,
    R.pool_idle_timeout_secs.max
);
const DOWNLOAD_TIMEOUT_REASON: &str = formatcp!(
    "must be {}..={} seconds",
    R.download_timeout_secs.min,
    R.download_timeout_secs.max
);
const MERGE_TIMEOUT_REASON: &str = formatcp!(
    "must be {}..={} seconds",
    R.merge_timeout_secs.min,
    R.merge_timeout_secs.max
);
const CONCURRENT_FRAGMENTS_REASON: &str = formatcp!(
    "must be {}..={} (caps peak transient memory under parallel fragment fetch)",
    R.concurrent_fragments.min,
    R.concurrent_fragments.max
);
const BUFFER_SIZE_REASON: &str = formatcp!(
    "must be {}..={} bytes (1 GiB)",
    R.buffer_size.min,
    R.buffer_size.max
);
const PARALLEL_THRESHOLD_REASON: &str = formatcp!(
    "must be {}..={} bytes (1 GiB)",
    R.parallel_threshold.min,
    R.parallel_threshold.max
);
const HLS_HEAD_PROBE_TIMEOUT_REASON: &str = formatcp!(
    "must be {}..={} seconds",
    R.hls_head_probe_timeout_secs.min,
    R.hls_head_probe_timeout_secs.max
);
const HLS_EXPANSION_TIMEOUT_REASON: &str = formatcp!(
    "must be {}..={} seconds",
    R.hls_expansion_timeout_secs.min,
    R.hls_expansion_timeout_secs.max
);

/// The ten network/download fields as a configuration layer holds them, under
/// their config-file keys: `None` = not set at this layer.
///
/// The projection both `Config::validate` and the desktop's
/// `AppSettings::validate_security` build so that
/// [`first_out_of_range`](Self::first_out_of_range) is the ONE range check —
/// neither validator can hold a bound the other lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetworkFields {
    /// `Config::socket_timeout`.
    pub socket_timeout: Option<u64>,
    /// `Config::read_timeout`.
    pub read_timeout: Option<u64>,
    /// `Config::pool_idle_timeout`.
    pub pool_idle_timeout: Option<u64>,
    /// `Config::download_timeout`.
    pub download_timeout: Option<u64>,
    /// `Config::merge_timeout`.
    pub merge_timeout: Option<u64>,
    /// `Config::concurrent_fragments`.
    pub concurrent_fragments: Option<u64>,
    /// `Config::buffer_size`, in bytes.
    pub buffer_size: Option<u64>,
    /// `Config::parallel_threshold`, in bytes.
    pub parallel_threshold: Option<u64>,
    /// `Config::hls_head_probe_timeout`.
    pub hls_head_probe_timeout: Option<u64>,
    /// `Config::hls_expansion_timeout`.
    pub hls_expansion_timeout: Option<u64>,
}

impl NetworkFields {
    /// The first field outside its owning range, as `(config key, reason)` —
    /// `None` when every set field is in range. Checked in declaration order.
    #[must_use]
    pub fn first_out_of_range(&self) -> Option<(&'static str, &'static str)> {
        let rows: [(&'static str, Option<u64>, NetworkRange, &'static str); 10] = [
            (
                "socket_timeout",
                self.socket_timeout,
                R.socket_timeout_secs,
                SOCKET_TIMEOUT_REASON,
            ),
            (
                "read_timeout",
                self.read_timeout,
                R.read_timeout_secs,
                READ_TIMEOUT_REASON,
            ),
            (
                "pool_idle_timeout",
                self.pool_idle_timeout,
                R.pool_idle_timeout_secs,
                POOL_IDLE_TIMEOUT_REASON,
            ),
            (
                "download_timeout",
                self.download_timeout,
                R.download_timeout_secs,
                DOWNLOAD_TIMEOUT_REASON,
            ),
            (
                "merge_timeout",
                self.merge_timeout,
                R.merge_timeout_secs,
                MERGE_TIMEOUT_REASON,
            ),
            (
                "concurrent_fragments",
                self.concurrent_fragments,
                R.concurrent_fragments,
                CONCURRENT_FRAGMENTS_REASON,
            ),
            (
                "buffer_size",
                self.buffer_size,
                R.buffer_size,
                BUFFER_SIZE_REASON,
            ),
            (
                "parallel_threshold",
                self.parallel_threshold,
                R.parallel_threshold,
                PARALLEL_THRESHOLD_REASON,
            ),
            (
                "hls_head_probe_timeout",
                self.hls_head_probe_timeout,
                R.hls_head_probe_timeout_secs,
                HLS_HEAD_PROBE_TIMEOUT_REASON,
            ),
            (
                "hls_expansion_timeout",
                self.hls_expansion_timeout,
                R.hls_expansion_timeout_secs,
                HLS_EXPANSION_TIMEOUT_REASON,
            ),
        ];
        rows.into_iter()
            .find(|(_, value, range, _)| value.is_some_and(|v| !range.contains(v)))
            .map(|(field, _, _, reason)| (field, reason))
    }
}

impl Default for EffectiveNetwork {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::EffectiveNetwork;

    /// Pins the documented values. This is the one place the literals may
    /// appear as a test oracle; every other test compares against
    /// `EffectiveNetwork::DEFAULT.<field>`.
    #[test]
    fn default_values_are_the_documented_ones() {
        let EffectiveNetwork {
            socket_timeout_secs,
            read_timeout_secs,
            pool_idle_timeout_secs,
            download_timeout_secs,
            merge_timeout_secs,
            concurrent_fragments,
            buffer_size,
            parallel_threshold,
            hls_head_probe_timeout_secs,
            hls_expansion_timeout_secs,
        } = EffectiveNetwork::DEFAULT;
        assert_eq!(socket_timeout_secs, 30);
        assert_eq!(read_timeout_secs, 60);
        assert_eq!(pool_idle_timeout_secs, 60);
        assert_eq!(download_timeout_secs, 3600);
        assert_eq!(merge_timeout_secs, 1800);
        assert_eq!(concurrent_fragments, 8);
        assert_eq!(buffer_size, 2 * 1024 * 1024);
        assert_eq!(parallel_threshold, 10 * 1024 * 1024);
        assert_eq!(hls_head_probe_timeout_secs, 5);
        assert_eq!(hls_expansion_timeout_secs, 60);
    }

    #[test]
    fn default_trait_is_the_const() {
        assert_eq!(EffectiveNetwork::default(), EffectiveNetwork::DEFAULT);
    }

    /// The desktop reads this over IPC (Task 2); the wire shape is the field
    /// names as written, with the sentinel `0` surviving as a plain number.
    #[test]
    fn serializes_field_names_verbatim() {
        let net = EffectiveNetwork {
            pool_idle_timeout_secs: 0,
            ..EffectiveNetwork::DEFAULT
        };
        let json = serde_json::to_value(net).expect("serialize");
        let fields = json.as_object().expect("a JSON object");
        assert_eq!(fields.get("socket_timeout_secs"), Some(&30.into()));
        assert_eq!(fields.get("pool_idle_timeout_secs"), Some(&0.into()));
        assert_eq!(fields.get("hls_head_probe_timeout_secs"), Some(&5.into()));
        assert_eq!(fields.get("hls_expansion_timeout_secs"), Some(&60.into()));
        assert_eq!(fields.len(), 10, "exactly ten fields on the wire");
    }
}

#[cfg(test)]
mod range_tests {
    use super::{EffectiveNetwork, NetworkFields, NetworkRange};

    /// Pins the documented bounds — the one place these literals may appear
    /// as a test oracle; `Config::validate` and the desktop read `RANGES`.
    #[test]
    fn ranges_are_the_documented_ones() {
        let r = EffectiveNetwork::RANGES;
        assert_eq!(
            (r.socket_timeout_secs.min, r.socket_timeout_secs.max),
            (1, 300)
        );
        assert_eq!((r.read_timeout_secs.min, r.read_timeout_secs.max), (1, 600));
        assert_eq!(
            (r.pool_idle_timeout_secs.min, r.pool_idle_timeout_secs.max),
            (0, 3600)
        );
        assert_eq!(
            (r.download_timeout_secs.min, r.download_timeout_secs.max),
            (1, 86400)
        );
        assert_eq!(
            (r.merge_timeout_secs.min, r.merge_timeout_secs.max),
            (1, 86400)
        );
        assert_eq!(
            (r.concurrent_fragments.min, r.concurrent_fragments.max),
            (1, 64)
        );
        assert_eq!(
            (r.buffer_size.min, r.buffer_size.max),
            (1, 1024 * 1024 * 1024)
        );
        assert_eq!(
            (r.parallel_threshold.min, r.parallel_threshold.max),
            (1, 1024 * 1024 * 1024)
        );
        assert_eq!(
            (
                r.hls_head_probe_timeout_secs.min,
                r.hls_head_probe_timeout_secs.max
            ),
            (1, 300)
        );
        assert_eq!(
            (
                r.hls_expansion_timeout_secs.min,
                r.hls_expansion_timeout_secs.max
            ),
            (1, 600)
        );
    }

    /// Every default sits inside its own range — a DEFAULT outside RANGES
    /// would be rejected the moment a user typed it back in.
    #[test]
    fn defaults_are_inside_their_ranges() {
        let d = EffectiveNetwork::DEFAULT;
        let r = EffectiveNetwork::RANGES;
        assert!(r.socket_timeout_secs.contains(d.socket_timeout_secs));
        assert!(r.read_timeout_secs.contains(d.read_timeout_secs));
        assert!(r.pool_idle_timeout_secs.contains(d.pool_idle_timeout_secs));
        assert!(r.download_timeout_secs.contains(d.download_timeout_secs));
        assert!(r.merge_timeout_secs.contains(d.merge_timeout_secs));
        assert!(
            r.concurrent_fragments
                .contains(d.concurrent_fragments as u64)
        );
        assert!(r.buffer_size.contains(d.buffer_size as u64));
        assert!(r.parallel_threshold.contains(d.parallel_threshold));
        assert!(
            r.hls_head_probe_timeout_secs
                .contains(d.hls_head_probe_timeout_secs)
        );
        assert!(
            r.hls_expansion_timeout_secs
                .contains(d.hls_expansion_timeout_secs)
        );
    }

    #[test]
    fn range_contains_is_inclusive() {
        let r = NetworkRange { min: 1, max: 300 };
        assert!(r.contains(1));
        assert!(r.contains(300));
        assert!(!r.contains(0));
        assert!(!r.contains(301));
    }

    /// The ONE check both validators call: names the config key, the reason
    /// cites the owner's bounds, `None` fields are skipped, and the first
    /// offender in table order wins.
    #[test]
    fn first_out_of_range_names_the_config_key_and_cites_the_bounds() {
        assert_eq!(NetworkFields::default().first_out_of_range(), None);
        let bad = NetworkFields {
            pool_idle_timeout: Some(3601),
            hls_expansion_timeout: Some(0),
            ..NetworkFields::default()
        };
        let (field, reason) = bad.first_out_of_range().expect("out of range");
        assert_eq!(field, "pool_idle_timeout");
        assert!(reason.contains("0..=3600"), "{reason}");
        let bad = NetworkFields {
            hls_expansion_timeout: Some(601),
            ..NetworkFields::default()
        };
        let (field, reason) = bad.first_out_of_range().expect("out of range");
        assert_eq!(field, "hls_expansion_timeout");
        assert!(reason.contains("1..=600"), "{reason}");
        let bad = NetworkFields {
            buffer_size: Some(1024 * 1024 * 1024 + 1),
            ..NetworkFields::default()
        };
        let (field, reason) = bad.first_out_of_range().expect("out of range");
        assert_eq!(field, "buffer_size");
        assert!(reason.contains("1..=1073741824"), "{reason}");
    }

    /// The desktop reads the ranges over IPC; the wire shape is the field
    /// names as written with `{min, max}` objects.
    #[test]
    fn ranges_serialize_field_names_verbatim() {
        let json = serde_json::to_value(EffectiveNetwork::RANGES).expect("serialize");
        let fields = json.as_object().expect("a JSON object");
        assert_eq!(fields.len(), 10, "exactly ten ranges on the wire");
        let pool = fields
            .get("pool_idle_timeout_secs")
            .and_then(|v| v.as_object())
            .expect("range object");
        assert_eq!(pool.get("min"), Some(&0.into()));
        assert_eq!(pool.get("max"), Some(&3600.into()));
    }
}
