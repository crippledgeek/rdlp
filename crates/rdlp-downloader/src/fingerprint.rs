//! The one FNV-1a-64 the resume sidecars fingerprint manifests with.
//!
//! Both sidecars need "did the manifest I resolved this list from change?"
//! answered from what the downloader is handed (HLS: `Fragment`s; DASH: a
//! `SegmentPlan`), without a copy of the manifest text. FNV-1a is not a
//! cryptographic hash and is not meant to be: the sidecar is the operator's
//! own file, and the fingerprint only has to separate two manifests that
//! differ. Every helper frames its input (a tag byte for `Option`, the
//! length for strings) so adjacent fields cannot alias.

/// FNV-1a 64-bit offset basis (Fowler/Noll/Vo, `fnv1a64`).
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV 64-bit prime.
const PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) struct Fnv1a64 {
    h: u64,
}

impl Fnv1a64 {
    pub(crate) const fn new() -> Self {
        Self { h: OFFSET_BASIS }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.h ^= u64::from(b);
            self.h = self.h.wrapping_mul(PRIME);
        }
    }

    pub(crate) fn feed_u64(&mut self, v: u64) {
        self.feed(&v.to_le_bytes());
    }

    pub(crate) fn feed_opt_u64(&mut self, v: Option<u64>) {
        match v {
            None => self.feed(&[0]),
            Some(v) => {
                self.feed(&[1]);
                self.feed_u64(v);
            }
        }
    }

    /// Like [`Self::feed_opt_u64`] but for a signed field (DASH
    /// `SegmentTimeline@r` is `i64`): `to_le_bytes` is lossless both ways, so
    /// this needs no `as`/`cast_unsigned` (the latter is 1.87+, above the
    /// workspace's 1.85 MSRV) and trips no `cast_sign_loss` lint.
    pub(crate) fn feed_opt_i64(&mut self, v: Option<i64>) {
        match v {
            None => self.feed(&[0]),
            Some(v) => {
                self.feed(&[1]);
                self.feed(&v.to_le_bytes());
            }
        }
    }

    pub(crate) fn feed_opt_range(&mut self, r: Option<(u64, u64)>) {
        match r {
            None => self.feed(&[0]),
            Some((a, b)) => {
                self.feed(&[1]);
                self.feed_u64(a);
                self.feed_u64(b);
            }
        }
    }

    /// `to_bits` so equal manifest text hashes equally; a re-encode that moves
    /// a segment boundary by a frame changes these bits.
    pub(crate) fn feed_opt_f64_bits(&mut self, v: Option<f64>) {
        self.feed_opt_u64(v.map(f64::to_bits));
    }

    pub(crate) fn feed_opt_str(&mut self, s: Option<&str>) {
        match s {
            None => self.feed(&[0]),
            Some(s) => {
                self.feed(&[1]);
                self.feed_u64(s.len() as u64);
                self.feed(s.as_bytes());
            }
        }
    }

    /// Path only — host and query are ignored so CDN host/token rotation does
    /// not break resume. A string that does not parse as an absolute URL
    /// (a relative fragment URI) is fed verbatim.
    ///
    /// Already `Option`-framed via `feed_opt_str`'s own tag byte (`Some`
    /// always precedes it): a caller wrapping an *optional* URL (e.g. an
    /// init segment that may be absent) uses [`Self::feed_opt_url_path`],
    /// not a second `feed(&[1])` around this — that would tag the value
    /// twice for no distinguishing benefit.
    pub(crate) fn feed_url_path(&mut self, url: &str) {
        match url::Url::parse(url) {
            Ok(u) => self.feed_opt_str(Some(u.path())),
            Err(_) => self.feed_opt_str(Some(url)),
        }
    }

    /// [`Self::feed_url_path`] for a field that may be absent (`None` ⇒
    /// `[0]`, matching every other `feed_opt_*` helper's framing).
    pub(crate) fn feed_opt_url_path(&mut self, url: Option<&str>) {
        match url {
            None => self.feed(&[0]),
            Some(u) => self.feed_url_path(u),
        }
    }

    pub(crate) const fn finish(&self) -> u64 {
        self.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feed_kind_separates_from_its_neighbour() {
        // Each helper must tag/frame its input so `None` != `Some(0)` and a
        // trailing/leading value cannot slide into the next field.
        let mut opt_none = Fnv1a64::new();
        opt_none.feed_opt_u64(None);
        let mut opt_zero = Fnv1a64::new();
        opt_zero.feed_opt_u64(Some(0));
        assert_ne!(opt_none.finish(), opt_zero.finish());

        let mut range_forward = Fnv1a64::new();
        range_forward.feed_opt_range(Some((0, 1)));
        let mut range_reversed = Fnv1a64::new();
        range_reversed.feed_opt_range(Some((1, 0)));
        assert_ne!(range_forward.finish(), range_reversed.finish());

        let mut duration_a = Fnv1a64::new();
        duration_a.feed_opt_f64_bits(Some(4.0));
        let mut duration_b = Fnv1a64::new();
        duration_b.feed_opt_f64_bits(Some(4.004));
        assert_ne!(duration_a.finish(), duration_b.finish());

        let mut split_ab_c = Fnv1a64::new();
        split_ab_c.feed_opt_str(Some("ab"));
        split_ab_c.feed_opt_str(Some("c"));
        let mut split_a_bc = Fnv1a64::new();
        split_a_bc.feed_opt_str(Some("a"));
        split_a_bc.feed_opt_str(Some("bc"));
        assert_ne!(
            split_ab_c.finish(),
            split_a_bc.finish(),
            "strings must be length-framed"
        );
    }

    #[test]
    fn url_path_ignores_host_and_query_but_keeps_relative_strings() {
        let mut a = Fnv1a64::new();
        a.feed_url_path("https://cdn1.example/v/seg-0.ts?t=A");
        let mut b = Fnv1a64::new();
        b.feed_url_path("https://cdn2.other/v/seg-0.ts?t=Z");
        assert_eq!(a.finish(), b.finish());
        let mut c = Fnv1a64::new();
        c.feed_url_path("seg-0.ts");
        let mut d = Fnv1a64::new();
        d.feed_url_path("seg-1.ts");
        assert_ne!(c.finish(), d.finish());
    }

    #[test]
    fn opt_url_path_frames_none_apart_from_an_empty_path() {
        // A relative "" URL falls back to `feed_opt_str(Some(""))`, which is
        // `[1, 0]` (tag, zero length) — `None`'s `[0]` must not collide with
        // it, and `feed_opt_url_path` must not double-tag `Some`.
        let mut none = Fnv1a64::new();
        none.feed_opt_url_path(None);
        let mut empty = Fnv1a64::new();
        empty.feed_opt_url_path(Some(""));
        assert_ne!(none.finish(), empty.finish());
    }

    #[test]
    fn matches_the_reference_fnv1a_vector() {
        // FNV-1a 64 of "a" is 0xaf63dc4c8601ec8c (Noll's published test vector).
        let mut h = Fnv1a64::new();
        h.feed(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
    }
}
