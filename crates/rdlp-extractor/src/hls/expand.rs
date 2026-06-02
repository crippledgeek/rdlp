//! HLS master/media playlist expansion into pre-resolved `Format` entries
//! with `Format.fragments` populated.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rdlp_types::{Format, Fragment};

/// Refusal/failure modes for [`expand_hls_url`].
///
/// On any error the caller should fall back to the legacy `Format` row
/// (manifest URL with `fragments = None`) so the downloader re-parses the
/// playlist at download time.
#[derive(Debug, thiserror::Error)]
pub enum HlsExpandError {
    /// Master playlist parsed cleanly but contained zero variants.
    #[error("master playlist has no variants")]
    NoVariants,
    /// Media playlist parsed cleanly but contained zero segments.
    #[error("variant playlist has no segments")]
    NoSegments,
    /// Media playlist segment count exceeds the configured cap.
    #[error("variant playlist has too many segments: {count} (max {max})")]
    TooManySegments {
        /// Observed segment count.
        count: usize,
        /// Configured maximum.
        max: usize,
    },
    /// EXT-X-KEY indicated an encryption method other than `NONE`.
    #[error("HLS encryption not supported in pre-resolved path: {0}")]
    Encrypted(String),
    /// Media playlist lacks EXT-X-ENDLIST (live / event stream).
    #[error("HLS live streams not supported in pre-resolved path")]
    LiveStream,
    /// HTTP fetch of master or media playlist failed.
    #[error("network: {0}")]
    Network(String),
    /// `m3u8_rs` could not parse the playlist body.
    #[error("parse: {0}")]
    Parse(String),
    /// Master playlist has more variants than `MAX_VARIANTS`.
    #[error("master playlist has too many variants: {count} (max {max})")]
    TooManyVariants {
        /// Actual variant count in the master playlist.
        count: usize,
        /// Configured maximum.
        max: usize,
    },

    /// Distinct `EXT-X-MAP` URIs in the playlist exceed `MAX_INIT_SEGMENTS`.
    /// Mitigates interleaved-init-URI fetch amplification.
    #[error("media playlist has too many distinct init segments: {count} (max {max})")]
    TooManyInitSegments {
        /// Distinct init URI count observed.
        count: usize,
        /// Configured maximum.
        max: usize,
    },

    /// `EXT-X-BYTERANGE` or `EXT-X-MAP:BYTERANGE` parsed to an empty or
    /// inverted range (e.g. `BYTERANGE="0@0"`). Caught at population time
    /// to avoid emitting `Range: bytes=0--1` (u64 underflow).
    #[error("invalid byte range: start={start}, end_exclusive={end_exclusive}")]
    InvalidByteRange {
        /// Resolved start offset.
        start: u64,
        /// Resolved end (exclusive).
        end_exclusive: u64,
    },
}

/// Cap on master playlist variant count.
const MAX_VARIANTS: usize = 50;
/// Cap on distinct EXT-X-MAP URIs in a single media playlist.
/// Mitigates interleaved-init-URI fetch amplification (`A,B,A,B,...`).
const MAX_INIT_SEGMENTS: usize = 50;
/// Cap on raw playlist body size (master or media).
const MAX_PLAYLIST_BYTES: usize = 8 * 1024 * 1024;

/// Validate a resolved URL (variant URI or segment URI from a playlist body).
///
/// Production behavior: delegate to `rdlp_security::validate_url_security`,
/// which rejects file://, javascript:, private hosts, and other SSRF-prone
/// targets.
///
/// Test behavior: allow http/https on `127.0.0.1` / `localhost` so mockito
/// servers (which bind to loopback by default) can drive integration tests.
/// All other URLs (other private hosts, non-http(s) schemes) still go
/// through the real validator. The bypass is `cfg(test)`-gated so production
/// builds compile without any loopback exemption.
fn validate_resolved_url(url: &str) -> Result<(), HlsExpandError> {
    #[cfg(test)]
    {
        if let Ok(parsed) = url::Url::parse(url) {
            let scheme_ok = matches!(parsed.scheme(), "http" | "https");
            let host_loopback = parsed.host_str().is_some_and(|h| {
                h == "127.0.0.1" || h == "localhost" || h == "[::1]" || h == "::1"
            });
            if scheme_ok && host_loopback {
                return Ok(());
            }
        }
    }
    rdlp_security::validate_url_security(url)
        .map_err(|e| HlsExpandError::Network(format!("URI rejected: {e}")))
}

/// Expand an HLS URL (master or media playlist) into pre-resolved `Format`
/// entries.
///
/// Auto-detects master vs media via `m3u8_rs::Playlist`. Master playlists
/// expand to one `Format` per variant; media playlists return a single-element
/// `Vec<Format>`.
///
/// Each output `Format` inherits codec/height/format_id/format_note metadata
/// from `seed`, then has `fragments = Some(...)` populated with absolute
/// fragment URLs (resolved against the *media* playlist URL per RFC 8216 §4.1).
///
/// # Errors
///
/// Returns `HlsExpandError` for refusal cases (encrypted, live, multi-init,
/// byte-ranged init, empty variants/segments, network/parse failures). The
/// caller should treat any error as a signal to keep the original `Format`
/// row (legacy fallback).
pub async fn expand_hls_url(
    seed: &Format,
    http: Arc<wreq::Client>,
) -> Result<Vec<Format>, HlsExpandError> {
    let headers = seed.http_headers.as_ref();
    // Seed origin (scheme + host + port) — used to gate cross-origin header
    // forwarding on variant fetches. `Url::origin()` returns Opaque for URLs
    // without a defined tuple origin (e.g. data:); Opaque values compare
    // not-equal, so the cross-origin check defaults to the safe "drop headers"
    // outcome when either side cannot be parsed.
    let seed_origin = url::Url::parse(&seed.url).ok().map(|u| u.origin());

    // Master fetch: always to seed.url — same origin by definition, headers safe.
    let bytes = fetch_playlist_bytes(&http, &seed.url, headers).await?;
    let playlist = m3u8_rs::parse_playlist_res(&bytes)
        .map_err(|e| HlsExpandError::Parse(format!("playlist: {e:?}")))?;

    match playlist {
        m3u8_rs::Playlist::MediaPlaylist(_) => {
            let f = expand_media_playlist(seed, &seed.url, &bytes)?;
            Ok(vec![f])
        }
        m3u8_rs::Playlist::MasterPlaylist(master) => {
            let base = url::Url::parse(&seed.url)
                .map_err(|e| HlsExpandError::Parse(format!("invalid master url: {e}")))?;
            // Single source of per-variant metadata (resolution, codecs,
            // frame-rate, audio-rendition groups), shared with the detect/size
            // path via `apply_variant_labels`. Includes EXT-X-MEDIA audio
            // renditions, not just EXT-X-STREAM-INF video variants.
            let variant_infos = crate::hls::variants::expand_master_variants(&master, &base);
            if variant_infos.is_empty() {
                return Err(HlsExpandError::NoVariants);
            }
            if variant_infos.len() > MAX_VARIANTS {
                return Err(HlsExpandError::TooManyVariants {
                    count: variant_infos.len(),
                    max: MAX_VARIANTS,
                });
            }
            let mut out = Vec::with_capacity(variant_infos.len());
            for vinfo in &variant_infos {
                let media_url = vinfo.media_playlist_url.clone();
                validate_resolved_url(&media_url)?;
                // Cross-origin header forwarding is suppressed: a malicious or
                // compromised master playlist whose variant URI points to an
                // attacker-controlled CDN must NOT receive Referer / Cookie /
                // Authorization or any other operator-set header. Only forward
                // when the variant's origin (scheme+host+port) matches the
                // seed's origin.
                let same_origin = match (&seed_origin, url::Url::parse(&media_url).ok()) {
                    (Some(a), Some(b)) => *a == b.origin(),
                    _ => false,
                };
                let variant_headers = if same_origin { headers } else { None };
                let media_bytes = fetch_playlist_bytes(&http, &media_url, variant_headers).await?;
                let mut f = expand_media_playlist(seed, &media_url, &media_bytes)?;
                // Point the row at its OWN media playlist (not the master) and
                // stamp per-variant labels (height/codecs/resolution-suffixed
                // format_id) so the downstream `detect_format_sizes_lazy` pass
                // enriches the complete row instead of re-fetching the master
                // and re-expanding into fragment-less rows (the xhamster
                // `hls-h264-url-2160p` bug).
                f.url = media_url;
                crate::hls::format_detection::apply_variant_labels(&mut f, vinfo, seed);
                // `apply_variant_labels` copies the seed's operator headers onto
                // the row (used by the downloader for SEGMENT fetches). For a
                // cross-origin variant, drop them — the same-origin gate above
                // already withheld them from the playlist fetch, and forwarding
                // Referer/Cookie/Auth to a cross-origin segment CDN is the same
                // leak (review LOW-1).
                if !same_origin {
                    f.http_headers = None;
                }
                out.push(f);
            }
            Ok(out)
        }
    }
}

/// Fetch a playlist body, forwarding the seed Format's `http_headers` (if any)
/// on the GET request. This is what lets sites like 9anime/Megacloud (which
/// require a `Referer` header on the master + variant playlist fetches)
/// successfully reach the per-variant fragment fast path. Without forwarding,
/// the fetch would 403 and `expand_hls_in_place` would silently fall back to
/// the legacy variant-URL download path, defeating the optimization.
async fn fetch_playlist_bytes(
    http: &wreq::Client,
    url: &str,
    headers: Option<&HashMap<String, String>>,
) -> Result<Vec<u8>, HlsExpandError> {
    let safe_url = rdlp_security::sanitize_for_logging(url);
    let mut req = http.get(url);
    if let Some(headers) = headers {
        for (k, v) in headers {
            req = req.header(k, v);
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| HlsExpandError::Network(format!("fetch {safe_url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(HlsExpandError::Network(format!(
            "fetch {safe_url}: status {}",
            resp.status()
        )));
    }
    if let Some(len) = resp.content_length()
        && len > MAX_PLAYLIST_BYTES as u64
    {
        return Err(HlsExpandError::Network(format!(
            "playlist body too large: {len} bytes (max {MAX_PLAYLIST_BYTES})"
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| HlsExpandError::Network(format!("read {safe_url}: {e}")))?;
    if bytes.len() > MAX_PLAYLIST_BYTES {
        return Err(HlsExpandError::Network(format!(
            "playlist body too large: {} bytes (max {MAX_PLAYLIST_BYTES})",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

/// Build per-media-playlist `Format` from parsed bytes + media-playlist URL.
///
/// Internal helper — used by `expand_hls_url` after fetching. Separated out
/// so refusal logic can be unit-tested without HTTP mocking.
fn expand_media_playlist(
    seed: &Format,
    media_playlist_url: &str,
    bytes: &[u8],
) -> Result<Format, HlsExpandError> {
    let playlist = m3u8_rs::parse_media_playlist_res(bytes)
        .map_err(|e| HlsExpandError::Parse(format!("media playlist: {e:?}")))?;

    // Encryption refusal — check every segment's `key` field.
    for seg in &playlist.segments {
        if let Some(key) = &seg.key {
            match &key.method {
                m3u8_rs::KeyMethod::None => {}
                m3u8_rs::KeyMethod::AES128 => {
                    return Err(HlsExpandError::Encrypted("AES-128".into()));
                }
                m3u8_rs::KeyMethod::SampleAES => {
                    return Err(HlsExpandError::Encrypted("SAMPLE-AES".into()));
                }
                m3u8_rs::KeyMethod::Other(s) => {
                    return Err(HlsExpandError::Encrypted(s.clone()));
                }
            }
        }
    }

    if !playlist.end_list {
        return Err(HlsExpandError::LiveStream);
    }
    if playlist.playlist_type == Some(m3u8_rs::MediaPlaylistType::Event) {
        return Err(HlsExpandError::LiveStream);
    }

    const MAX_SEGMENTS: usize = 10_000;
    if playlist.segments.is_empty() {
        return Err(HlsExpandError::NoSegments);
    }
    if playlist.segments.len() > MAX_SEGMENTS {
        return Err(HlsExpandError::TooManySegments {
            count: playlist.segments.len(),
            max: MAX_SEGMENTS,
        });
    }

    let base = url::Url::parse(media_playlist_url)
        .map_err(|e| HlsExpandError::Parse(format!("invalid media playlist url: {e}")))?;

    let resolve = |raw: &str| -> Result<String, HlsExpandError> {
        let resolved = base
            .join(raw)
            .map_or_else(|_| raw.to_string(), |u| u.to_string());
        validate_resolved_url(&resolved)?;
        Ok(resolved)
    };

    // Helper: m3u8_rs ByteRange { length, offset: Option } → (start, end_exclusive).
    // Per RFC 8216 §4.3.2.2, a missing offset means "contiguous from previous
    // segment's end in the same resource." We track that via prev_end.
    let convert_br =
        |br: &m3u8_rs::ByteRange, prev_end: u64| -> Result<(u64, u64), HlsExpandError> {
            let start = br.offset.unwrap_or(prev_end);
            let end_exclusive = start.saturating_add(br.length);
            if end_exclusive == 0 || end_exclusive <= start {
                return Err(HlsExpandError::InvalidByteRange {
                    start,
                    end_exclusive,
                });
            }
            Ok((start, end_exclusive))
        };

    // Track distinct init URIs across the playlist (security: cap at
    // MAX_INIT_SEGMENTS to defeat interleaved-init-URI amplification).
    let mut distinct_inits: HashSet<String> = HashSet::new();

    let mut fragments: Vec<Fragment> = Vec::with_capacity(playlist.segments.len());
    let mut prev_seg_end: u64 = 0;
    let mut current_init_url: Option<String> = None;
    let mut current_init_br: Option<(u64, u64)> = None;

    for seg in &playlist.segments {
        // EXT-X-MAP transition: m3u8_rs sets seg.map on every segment the
        // most-recent EXT-X-MAP applies to (no propagation needed on our
        // side). We update current_init_* and apply the cap.
        if let Some(map) = seg.map.as_ref() {
            let map_url = resolve(&map.uri)?;
            // Track distinct init URIs and apply the cap. insert returns
            // true if the value was newly inserted (i.e., a new distinct URI).
            if distinct_inits.insert(map_url.clone()) && distinct_inits.len() > MAX_INIT_SEGMENTS {
                return Err(HlsExpandError::TooManyInitSegments {
                    count: distinct_inits.len(),
                    max: MAX_INIT_SEGMENTS,
                });
            }
            let map_br = if let Some(br) = map.byte_range.as_ref() {
                Some(convert_br(br, 0)?)
            } else {
                None
            };
            current_init_url = Some(map_url);
            current_init_br = map_br;
        }

        let url = resolve(&seg.uri)?;
        let byte_range = if let Some(br) = seg.byte_range.as_ref() {
            let r = convert_br(br, prev_seg_end)?;
            prev_seg_end = r.1;
            Some(r)
        } else {
            None
        };

        fragments.push(Fragment {
            url,
            byte_range,
            init_url: current_init_url.clone(),
            init_byte_range: current_init_br,
            duration: Some(f64::from(seg.duration)),
            filesize: None,
        });
    }

    // Sum per-segment EXTINF durations so the row carries an accurate total
    // duration even when the detect/size pass is skipped (it is, for
    // already-fragment-resolved rows). Drives %(duration)s + filesize_approx.
    let total_duration: f64 = fragments.iter().filter_map(|f| f.duration).sum();

    let mut out = seed.clone();
    out.fragments = Some(fragments);
    out.fragment_base_url = None; // URLs already absolutized.
    if total_duration > 0.0 {
        out.duration = Some(total_duration);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::DownloadProtocol;

    #[test]
    fn hls_expand_error_displays() {
        let e = HlsExpandError::Encrypted("AES-128".into());
        assert_eq!(
            e.to_string(),
            "HLS encryption not supported in pre-resolved path: AES-128"
        );
    }

    fn seed() -> Format {
        Format::new(
            "hls",
            "https://h.com/master.m3u8",
            "m3u8",
            DownloadProtocol::M3u8,
        )
    }

    const MEDIA_AES128: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=AES-128,URI=\"https://h.com/key\"
#EXTINF:6.0,
seg-1.ts
#EXT-X-ENDLIST
";

    const MEDIA_SAMPLE_AES: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://h.com/key\"
#EXTINF:6.0,
seg-1.ts
#EXT-X-ENDLIST
";

    const MEDIA_KEY_NONE: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=NONE
#EXTINF:6.0,
seg-1.ts
#EXT-X-ENDLIST
";

    #[test]
    fn refuses_aes128_encryption() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_AES128)
            .expect_err("must refuse AES-128");
        assert!(matches!(err, HlsExpandError::Encrypted(ref s) if s == "AES-128"));
    }

    #[test]
    fn refuses_sample_aes_encryption() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_SAMPLE_AES)
            .expect_err("must refuse SAMPLE-AES");
        assert!(matches!(err, HlsExpandError::Encrypted(ref s) if s == "SAMPLE-AES"));
    }

    #[test]
    fn key_method_none_is_not_encrypted() {
        // METHOD=NONE means "no encryption" — must succeed.
        let f = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_KEY_NONE)
            .expect("METHOD=NONE must succeed");
        assert!(f.fragments.is_some());
    }

    const MEDIA_LIVE_NO_ENDLIST: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
seg-1.ts
";

    const MEDIA_PLAYLIST_TYPE_EVENT: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-PLAYLIST-TYPE:EVENT
#EXTINF:6.0,
seg-1.ts
#EXT-X-ENDLIST
";

    #[test]
    fn refuses_missing_endlist() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_LIVE_NO_ENDLIST)
            .expect_err("must refuse live");
        assert!(matches!(err, HlsExpandError::LiveStream));
    }

    #[test]
    fn refuses_playlist_type_event() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_PLAYLIST_TYPE_EVENT)
            .expect_err("must refuse EVENT");
        assert!(matches!(err, HlsExpandError::LiveStream));
    }

    const MEDIA_MULTI_INIT: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI=\"init-a.m4s\"
#EXTINF:6.0,
seg-1.m4s
#EXT-X-MAP:URI=\"init-b.m4s\"
#EXTINF:6.0,
seg-2.m4s
#EXT-X-ENDLIST
";

    const MEDIA_BYTE_RANGED_INIT: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI=\"init.m4s\",BYTERANGE=\"100@200\"
#EXTINF:6.0,
seg-1.m4s
#EXT-X-ENDLIST
";

    #[test]
    fn too_many_init_segments_rejected() {
        // Generate a playlist with 51 distinct EXT-X-MAP URIs.
        let mut body = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n");
        for i in 0..51 {
            body.push_str(&format!(
                "#EXT-X-MAP:URI=\"init-{i}.mp4\"\n#EXTINF:6.0,\nseg-{i}.m4s\n"
            ));
        }
        body.push_str("#EXT-X-ENDLIST\n");

        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", body.as_bytes())
            .expect_err("must refuse >MAX_INIT_SEGMENTS distinct inits");
        assert!(matches!(
            err,
            HlsExpandError::TooManyInitSegments { count: 51, max: 50 }
        ));
    }

    #[test]
    fn multiple_distinct_init_segments_no_longer_refused() {
        // Regression guard: prior behavior was HlsExpandError::MultipleInitSegments;
        // now multi-init is supported per-fragment.
        let f = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_MULTI_INIT)
            .expect("multi-init must be accepted post-#267");
        let frag = f.fragments.as_ref().unwrap();
        assert_ne!(
            frag[0].init_url, frag[1].init_url,
            "different inits across stream"
        );
    }

    /// Multi-init: two distinct EXT-X-MAP URIs in one playlist.
    /// Per RFC 8216 §4.4.2.5 each EXT-X-MAP applies to subsequent
    /// segments until the next one. Expansion populates the appropriate
    /// init_url on each Fragment.
    const MEDIA_MULTI_INIT_3SEG: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI=\"init-a.mp4\"
#EXTINF:6.0,
seg-1.m4s
#EXTINF:6.0,
seg-2.m4s
#EXT-X-MAP:URI=\"init-b.mp4\"
#EXTINF:6.0,
seg-3.m4s
#EXT-X-ENDLIST
";

    #[test]
    fn multi_init_populates_per_fragment_init_url() {
        let f = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_MULTI_INIT_3SEG)
            .expect("must accept multi-init");
        let frag = f.fragments.as_ref().unwrap();
        assert_eq!(frag.len(), 3);
        assert_eq!(
            frag[0].init_url.as_deref(),
            Some("https://h.com/init-a.mp4")
        );
        assert_eq!(
            frag[1].init_url.as_deref(),
            Some("https://h.com/init-a.mp4")
        );
        assert_eq!(
            frag[2].init_url.as_deref(),
            Some("https://h.com/init-b.mp4")
        );
    }

    #[test]
    fn byte_range_init_no_longer_refused() {
        // Regression guard: prior behavior was HlsExpandError::ByteRangedInit;
        // now byte-range init is supported and the field is populated.
        let f = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_BYTE_RANGED_INIT)
            .expect("byte-range init must be accepted post-#267");
        assert!(f.fragments.as_ref().unwrap()[0].init_byte_range.is_some());
    }

    const MEDIA_NO_SEGMENTS: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-ENDLIST
";

    #[test]
    fn refuses_zero_segments() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_NO_SEGMENTS)
            .expect_err("must refuse empty");
        assert!(matches!(err, HlsExpandError::NoSegments));
    }

    #[test]
    fn refuses_too_many_segments() {
        let mut body = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n");
        for i in 1..=10_001 {
            body.push_str(&format!("#EXTINF:1.0,\nseg-{i}.ts\n"));
        }
        body.push_str("#EXT-X-ENDLIST\n");
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", body.as_bytes())
            .expect_err("must refuse oversized");
        assert!(matches!(
            err,
            HlsExpandError::TooManySegments {
                count: 10_001,
                max: 10_000
            }
        ));
    }

    const MEDIA_SIMPLE: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
seg-1.ts
#EXTINF:6.0,
seg-2.ts
#EXT-X-ENDLIST
";

    const MEDIA_WITH_INIT: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI=\"init.m4s\"
#EXTINF:6.0,
seg-1.m4s
#EXTINF:6.0,
seg-2.m4s
#EXT-X-ENDLIST
";

    const MEDIA_ABS_URIS: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
https://cdn.h.com/abs-1.ts
#EXTINF:6.0,
https://cdn.h.com/abs-2.ts
#EXT-X-ENDLIST
";

    #[test]
    fn relative_uris_resolve_against_media_playlist_url() {
        let f = expand_media_playlist(&seed(), "https://h.com/v/720/a.m3u8", MEDIA_SIMPLE)
            .expect("simple playlist must succeed");
        let frags = f.fragments.expect("fragments populated");
        assert_eq!(frags.len(), 2);
        assert_eq!(frags[0].url, "https://h.com/v/720/seg-1.ts");
        assert_eq!(frags[1].url, "https://h.com/v/720/seg-2.ts");
        assert_eq!(frags[0].duration, Some(6.0));
    }

    #[test]
    fn absolute_uris_preserved_unchanged() {
        let f = expand_media_playlist(&seed(), "https://h.com/v/720/a.m3u8", MEDIA_ABS_URIS)
            .expect("absolute uris");
        let frags = f.fragments.expect("fragments populated");
        assert_eq!(frags[0].url, "https://cdn.h.com/abs-1.ts");
        assert_eq!(frags[1].url, "https://cdn.h.com/abs-2.ts");
    }

    #[test]
    fn init_segment_stored_as_per_fragment_init_url() {
        // Previously init was folded as Fragment[0] sentinel (duration=None, url=init.m4s).
        // Now each data Fragment carries init_url + init_byte_range refs instead.
        let f = expand_media_playlist(&seed(), "https://h.com/v/720/a.m3u8", MEDIA_WITH_INIT)
            .expect("init+segments");
        let frags = f.fragments.expect("fragments populated");
        assert_eq!(
            frags.len(),
            2,
            "2 data segments only (no sentinel init Fragment)"
        );
        assert_eq!(frags[0].url, "https://h.com/v/720/seg-1.m4s");
        assert_eq!(frags[0].duration, Some(6.0));
        assert_eq!(
            frags[0].init_url.as_deref(),
            Some("https://h.com/v/720/init.m4s")
        );
        assert_eq!(frags[0].init_byte_range, None);
        assert_eq!(frags[1].url, "https://h.com/v/720/seg-2.m4s");
        assert_eq!(
            frags[1].init_url.as_deref(),
            Some("https://h.com/v/720/init.m4s")
        );
    }

    fn seed_with_metadata() -> Format {
        let mut f = Format::new(
            "hls",
            "https://h.com/master.m3u8",
            "m3u8",
            DownloadProtocol::M3u8,
        );
        f.vcodec = rdlp_types::Codec::from("h264".to_string());
        f.acodec = rdlp_types::Codec::from("aac".to_string());
        f.height = Some(720);
        f.format_note = Some("HLS".to_string());
        f
    }

    const MASTER_ONE_VARIANT: &str = "\
#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=720x480
v720.m3u8
";

    const MASTER_THREE_VARIANTS: &str = "\
#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=480000,RESOLUTION=480x270
v240.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=854x480
v480.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2560000,RESOLUTION=1280x720
v720.m3u8
";

    const MEDIA_PLAIN: &str = "\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
seg-1.ts
#EXTINF:6.0,
seg-2.ts
#EXT-X-ENDLIST
";

    /// Security (review LOW-1): operator headers (Referer/Cookie/Auth) must NOT
    /// be stamped on a variant row whose media playlist lives on a DIFFERENT
    /// origin than the seed — otherwise the downloader would forward them to a
    /// cross-origin segment CDN. The same-origin gate already drops them for the
    /// playlist FETCH; this guards the row's `http_headers` used for SEGMENT
    /// fetches. Two mockito servers = two ports = two origins (cross-origin).
    #[tokio::test]
    async fn cross_origin_variant_strips_operator_headers() {
        let master_srv = mockito::Server::new_async().await;
        let mut variant_srv = mockito::Server::new_async().await;
        let _v = variant_srv
            .mock("GET", "/v.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;
        let variant_url = format!("{}/v.m3u8", variant_srv.url());

        // Master served via a manual mock referencing the cross-origin variant.
        let mut master_srv = master_srv;
        let master_body = format!(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=1280x720\n{variant_url}\n"
        );
        let _m = master_srv
            .mock("GET", "/master.m3u8")
            .with_body(master_body)
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/master.m3u8", master_srv.url());
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://site.example/page".to_string(),
        );
        s.http_headers = Some(headers);

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect("expand ok");

        assert_eq!(out.len(), 1);
        assert!(
            out[0].http_headers.is_none(),
            "operator headers must NOT be forwarded to a cross-origin variant; got {:?}",
            out[0].http_headers
        );
    }

    /// Positive counterpart to the cross-origin test: a same-origin variant
    /// (served by the same mockito server) MUST retain the seed's operator
    /// headers so the downloader can fetch segments with the required Referer.
    #[tokio::test]
    async fn same_origin_variant_preserves_operator_headers() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body(MASTER_ONE_VARIANT)
            .create_async()
            .await;
        let _variant = server
            .mock("GET", "/v720.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Referer".to_string(),
            "https://site.example/page".to_string(),
        );
        s.http_headers = Some(headers);

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect("expand ok");

        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0]
                .http_headers
                .as_ref()
                .and_then(|h| h.get("Referer"))
                .map(String::as_str),
            Some("https://site.example/page"),
            "same-origin variant must retain operator headers for segment fetches"
        );
    }

    #[tokio::test]
    async fn master_with_one_variant_expands_to_one_format() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body(MASTER_ONE_VARIANT)
            .create_async()
            .await;
        let _variant = server
            .mock("GET", "/v720.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect("expand ok");

        assert_eq!(out.len(), 1);
        assert!(out[0].fragments.is_some());
        assert_eq!(out[0].fragments.as_ref().unwrap().len(), 2);
    }

    /// Regression guard (xhamster live `hls-h264-url-2160p`): master expansion
    /// must emit COMPLETE per-variant rows — each row's `url` points at its own
    /// variant media playlist (NOT the master), `height` is the per-variant
    /// resolution, and `format_id` carries the `-{h}p` suffix — all alongside
    /// the pre-resolved `fragments`. Pre-fix, `expand_media_playlist` left
    /// `url = master` and inherited the seed's generic label, so the downstream
    /// `detect_format_sizes_lazy` pass re-fetched the master, re-expanded, and
    /// replaced these rows with fragment-less ones → the downloader error.
    #[tokio::test]
    async fn master_expansion_emits_complete_labeled_rows() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body(MASTER_THREE_VARIANTS)
            .create_async()
            .await;
        let _v240 = server
            .mock("GET", "/v240.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;
        let _v480 = server
            .mock("GET", "/v480.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v720.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;

        let mut s = seed();
        s.format_id = "hls-h264-url".to_string();
        s.url = format!("{}/master.m3u8", server.url());

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect("expand ok");

        assert_eq!(out.len(), 3, "three variants → three rows");
        for f in &out {
            assert!(
                f.fragments.is_some(),
                "every expanded row must carry fragments (row {})",
                f.format_id
            );
        }

        // The 720p variant: url must point at its OWN media playlist, height set
        // per-variant, format_id resolution-suffixed.
        let r720 = out
            .iter()
            .find(|f| f.height == Some(720))
            .expect("a 720p row must be present (per-variant height)");
        assert!(
            r720.url.ends_with("/v720.m3u8"),
            "row url must point at the variant media playlist, not the master; got {}",
            r720.url
        );
        assert_eq!(
            r720.format_id, "hls-h264-url-720p",
            "format_id must carry the resolution suffix"
        );
    }

    #[tokio::test]
    async fn master_with_three_variants_expands_to_three_formats() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body(MASTER_THREE_VARIANTS)
            .create_async()
            .await;
        for v in ["v240.m3u8", "v480.m3u8", "v720.m3u8"] {
            let _ = server
                .mock("GET", format!("/{v}").as_str())
                .with_body(MEDIA_PLAIN)
                .create_async()
                .await;
        }

        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect("expand ok");

        assert_eq!(out.len(), 3);
        for f in &out {
            assert!(f.fragments.is_some());
        }
    }

    #[tokio::test]
    async fn direct_media_playlist_returns_single_format() {
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/media.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/media.m3u8", server.url());

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect("expand ok");

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].fragments.as_ref().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn seed_metadata_inherited() {
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/media.m3u8")
            .with_body(MEDIA_PLAIN)
            .create_async()
            .await;

        let mut s = seed_with_metadata();
        s.url = format!("{}/media.m3u8", server.url());

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect("expand ok");

        assert_eq!(out[0].vcodec.as_str(), Some("h264"));
        assert_eq!(out[0].acodec.as_str(), Some("aac"));
        assert_eq!(out[0].height, Some(720));
        assert_eq!(out[0].format_id, "hls");
        assert_eq!(out[0].format_note.as_deref(), Some("HLS"));
    }

    const MEDIA_FILE_SCHEME_INJECTION: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
file:///etc/passwd
#EXT-X-ENDLIST
";

    const MEDIA_PRIVATE_HOST_INJECTION: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
http://169.254.169.254/latest/meta-data/
#EXT-X-ENDLIST
";

    const MEDIA_RFC1918_INJECTION: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
http://10.0.0.1/admin
#EXT-X-ENDLIST
";

    #[test]
    fn refuses_file_scheme_segment_uri() {
        let err =
            expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_FILE_SCHEME_INJECTION)
                .expect_err("must refuse file:// segment");
        assert!(matches!(err, HlsExpandError::Network(_)));
    }

    #[test]
    fn refuses_link_local_segment_uri() {
        let err = expand_media_playlist(
            &seed(),
            "https://h.com/v.m3u8",
            MEDIA_PRIVATE_HOST_INJECTION,
        )
        .expect_err("must refuse link-local segment");
        assert!(matches!(err, HlsExpandError::Network(_)));
    }

    #[test]
    fn refuses_rfc1918_segment_uri() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_RFC1918_INJECTION)
            .expect_err("must refuse RFC1918 segment");
        assert!(matches!(err, HlsExpandError::Network(_)));
    }

    #[tokio::test]
    async fn refuses_master_with_file_scheme_variant() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1280000\nfile:///etc/passwd\n")
            .create_async()
            .await;
        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());
        let http = std::sync::Arc::new(wreq::Client::new());
        let err = expand_hls_url(&s, http)
            .await
            .expect_err("must refuse file:// variant");
        assert!(matches!(err, HlsExpandError::Network(_)));
    }

    #[tokio::test]
    async fn refuses_master_with_too_many_variants() {
        let mut body = String::from("#EXTM3U\n");
        for i in 1..=51 {
            body.push_str(&format!("#EXT-X-STREAM-INF:BANDWIDTH={i}000\nv{i}.m3u8\n"));
        }
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body(body)
            .create_async()
            .await;
        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());
        let http = std::sync::Arc::new(wreq::Client::new());
        let err = expand_hls_url(&s, http)
            .await
            .expect_err("must refuse 51 variants");
        assert!(matches!(
            err,
            HlsExpandError::TooManyVariants { count: 51, max: 50 }
        ));
    }

    #[tokio::test]
    async fn empty_master_playlist_errors() {
        let mut server = mockito::Server::new_async().await;
        let _master = server
            .mock("GET", "/master.m3u8")
            .with_body("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n")
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());

        let http = std::sync::Arc::new(wreq::Client::new());
        let err = expand_hls_url(&s, http).await.expect_err("must error");
        // m3u8_rs ambiguously classifies a body with no STREAM-INF and no
        // segments. Depending on tags present it surfaces as either a master
        // playlist (NoVariants), a media playlist with zero segments
        // (NoSegments), or a media playlist missing ENDLIST (LiveStream).
        // Any of the three is an acceptable refusal here.
        assert!(matches!(
            err,
            HlsExpandError::NoSegments | HlsExpandError::NoVariants | HlsExpandError::LiveStream
        ));
    }

    /// Regression guard for #258 — `expand_hls_url` MUST forward the seed
    /// Format's `http_headers` (e.g. `Referer`) on BOTH the master and
    /// variant playlist GETs. Without this, sites like 9anime/Megacloud
    /// reject the master fetch with 403 and `expand_hls_in_place` silently
    /// falls back to the legacy variant-URL path, defeating the optimization.
    /// The mockito matcher rejects requests missing the header, turning
    /// "header dropped" into a Network error in this test.
    #[tokio::test]
    async fn expand_forwards_seed_http_headers_on_master_and_variant_fetches() {
        use mockito::Matcher;

        let mut server = mockito::Server::new_async().await;

        let _master = server
            .mock("GET", "/master.m3u8")
            .match_header("Referer", "https://megacloud.tv/embed-2/e-1/abc?k=1")
            .with_body(MASTER_ONE_VARIANT)
            .expect(1)
            .create_async()
            .await;

        let _variant = server
            .mock("GET", "/v720.m3u8")
            .match_header("Referer", "https://megacloud.tv/embed-2/e-1/abc?k=1")
            .with_body(MEDIA_PLAIN)
            .expect(1)
            .create_async()
            .await;

        // A "missed" mock fires a 501 if mockito sees a request that doesn't
        // match the headers above — turning a regression into a fetch error.
        let _master_unmatched = server
            .mock("GET", Matcher::Any)
            .with_status(501)
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());
        let mut h = HashMap::new();
        h.insert(
            "Referer".to_string(),
            "https://megacloud.tv/embed-2/e-1/abc?k=1".to_string(),
        );
        s.http_headers = Some(h);

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http)
            .await
            .expect("expand must succeed when Referer is forwarded");
        assert_eq!(out.len(), 1);
        assert!(out[0].fragments.is_some());
        // Seed header survives onto the expanded row via seed.clone().
        assert_eq!(
            out[0]
                .http_headers
                .as_ref()
                .and_then(|h| h.get("Referer"))
                .map(String::as_str),
            Some("https://megacloud.tv/embed-2/e-1/abc?k=1")
        );
    }

    /// Security regression guard for #258 — cross-host variant fetches MUST
    /// NOT receive the seed's `http_headers`. A malicious master playlist
    /// whose variant URI points to an attacker CDN should not exfiltrate
    /// the operator's Referer/Cookie/Authorization. The mockito matcher
    /// here expects the variant fetch to arrive WITHOUT a Referer header.
    /// If header forwarding to cross-host variants regresses, the matcher
    /// misses and the catch-all 501 fires, surfacing as Network error.
    #[tokio::test]
    async fn expand_does_not_forward_seed_headers_to_cross_host_variant() {
        use mockito::Matcher;

        // Master server — accepts the Referer header (same host as seed).
        let mut master_server = mockito::Server::new_async().await;
        // Variant server — different host (loopback but different port =
        // different origin). The matcher REJECTS any request carrying a
        // Referer header by requiring its absence.
        let mut variant_server = mockito::Server::new_async().await;

        // Body of the master playlist points the variant at the OTHER server
        // (cross-host). Note: build the absolute URL pointing at variant_server.
        let variant_abs_url = format!("{}/v720.m3u8", variant_server.url());
        let master_body = format!(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=720x480\n{variant_abs_url}\n"
        );

        let _master = master_server
            .mock("GET", "/master.m3u8")
            .match_header("Referer", "https://operator.example.com/page")
            .with_body(master_body)
            .expect(1)
            .create_async()
            .await;

        // Cross-host variant: must NOT see Referer.
        let _variant = variant_server
            .mock("GET", "/v720.m3u8")
            .match_header("Referer", Matcher::Missing)
            .with_body(MEDIA_PLAIN)
            .expect(1)
            .create_async()
            .await;

        // Catch-all on variant_server: 501 if Referer DID arrive.
        let _variant_unmatched = variant_server
            .mock("GET", Matcher::Any)
            .with_status(501)
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/master.m3u8", master_server.url());
        let mut h = HashMap::new();
        h.insert(
            "Referer".to_string(),
            "https://operator.example.com/page".to_string(),
        );
        s.http_headers = Some(h);

        let http = std::sync::Arc::new(wreq::Client::new());
        let out = expand_hls_url(&s, http).await.expect(
            "expand must succeed: master sees Referer, variant on different host sees nothing",
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].fragments.is_some());
    }

    #[test]
    fn too_many_init_segments_displays() {
        let e = HlsExpandError::TooManyInitSegments { count: 51, max: 50 };
        assert_eq!(
            e.to_string(),
            "media playlist has too many distinct init segments: 51 (max 50)"
        );
    }

    #[test]
    fn invalid_byte_range_displays() {
        let e = HlsExpandError::InvalidByteRange {
            start: 10,
            end_exclusive: 5,
        };
        assert_eq!(
            e.to_string(),
            "invalid byte range: start=10, end_exclusive=5"
        );
    }

    /// Byte-range EXT-X-MAP populates Fragment.init_byte_range and
    /// the conversion from m3u8_rs's (length, offset) representation
    /// to rdlp's (start, end_exclusive) is correct.
    const MEDIA_BYTE_RANGE_INIT: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"1024@2048\"
#EXTINF:6.0,
seg-1.m4s
#EXT-X-ENDLIST
";

    #[test]
    fn byte_range_init_populates_init_byte_range() {
        let f = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_BYTE_RANGE_INIT)
            .expect("must accept byte-range init");
        let frag = f.fragments.as_ref().expect("fragments populated");
        assert_eq!(
            frag.len(),
            1,
            "one data segment, init folded into Fragment.init_*"
        );
        assert_eq!(frag[0].init_url.as_deref(), Some("https://h.com/init.mp4"));
        // m3u8_rs ByteRange { length: 1024, offset: Some(2048) }
        // → start = 2048, end_exclusive = 2048 + 1024 = 3072
        assert_eq!(frag[0].init_byte_range, Some((2048, 3072)));
        assert_eq!(frag[0].url, "https://h.com/seg-1.m4s");
    }

    /// Security regression guard: an EXT-X-MAP URI pointing at a private
    /// host (e.g. metadata service) MUST be refused at expand time, not
    /// silently included in Fragment.init_url and exfiltrated at fetch.
    const MEDIA_PRIVATE_INIT: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI=\"http://169.254.169.254/latest/meta-data/\"
#EXTINF:6.0,
seg-1.m4s
#EXT-X-ENDLIST
";

    #[test]
    fn private_host_init_url_rejected_by_validate_resolved_url() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_PRIVATE_INIT)
            .expect_err("private-host init URI must be refused");
        // validate_resolved_url returns HlsExpandError::Network with a sanitized message.
        assert!(matches!(err, HlsExpandError::Network(_)));
    }

    /// Negative companion to the test above — proves the test infrastructure
    /// is real: when the seed has NO `http_headers`, the master fetch hits
    /// the unmatched-mock 501 path and `expand_hls_url` returns Network err.
    #[tokio::test]
    async fn expand_without_seed_headers_fails_when_server_requires_them() {
        use mockito::Matcher;

        let mut server = mockito::Server::new_async().await;

        // Master mock requires Referer.
        let _master = server
            .mock("GET", "/master.m3u8")
            .match_header("Referer", Matcher::Regex(r"^https://".to_string()))
            .with_body(MASTER_ONE_VARIANT)
            .create_async()
            .await;

        // Catch-all returns 501 when Referer is absent.
        let _unmatched = server
            .mock("GET", Matcher::Any)
            .with_status(501)
            .create_async()
            .await;

        let mut s = seed();
        s.url = format!("{}/master.m3u8", server.url());
        s.http_headers = None;

        let http = std::sync::Arc::new(wreq::Client::new());
        let err = expand_hls_url(&s, http)
            .await
            .expect_err("master fetch must fail without Referer");
        assert!(matches!(err, HlsExpandError::Network(_)));
    }
}
