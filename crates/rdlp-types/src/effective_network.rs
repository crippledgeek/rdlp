//! The materialised network/download settings — one owner for their defaults.

use serde::Serialize;

/// The resolved network and download settings a runtime consumer reads.
///
/// rdlp layers these nine values three deep, and every layer but the last
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
/// `DEFAULT` is the *only* place the nine default values live: rdlp-http,
/// rdlp-downloader and rdlp-extractor read them from here rather than
/// carrying a copy, so a value cannot drift between the crate that documents
/// a default and the one that applies it (#611). This struct is also the
/// payload intended for the desktop GUI's settings placeholders — served
/// over IPC by an `effective_network` command — so the GUI need not hold a
/// fifth copy either.
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
}

impl EffectiveNetwork {
    /// The nine network/download defaults. The single owner — no other crate
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
    };
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
        assert_eq!(fields.len(), 9, "exactly nine fields on the wire");
    }
}
