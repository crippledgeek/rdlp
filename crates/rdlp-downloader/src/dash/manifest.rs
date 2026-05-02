//! MPD parser. Wraps `dash_mpd::parse` and projects the relevant subset
//! into rdlp-internal types.
//!
//! Refuses live (`@type="dynamic"`) and DRM-protected MPDs. Multi-period
//! MPDs parse with a warning; only the first period is consumed (Task 3+
//! may revisit). The actual `SegmentPlan` is attached in Task 3 — this
//! module only resolves representation selection and BaseURL chains.

// DASH proper nouns (MPD, AdaptationSet, Representation, BaseURL, Period)
// appear extensively in doc-comments. The pedantic doc_markdown lint flags
// them as bare CamelCase identifiers, which is noise for a module whose
// vocabulary is fixed by an external spec.
#![allow(clippy::doc_markdown)]

use std::time::Duration;

use url::Url;

use crate::dash::errors::DashError;

/// Result of parsing an MPD manifest. Owns enough state to drive segment
/// downloads in a later task — the actual `SegmentPlan` is attached in Task 3.
#[derive(Debug)]
pub struct ParsedManifest {
    /// Effective duration of the (first) period. Falls back to
    /// `mpd@mediaPresentationDuration`, then to `Duration::ZERO`.
    pub period_duration: Duration,
    /// Selected video representation (highest bandwidth in the first period).
    pub video: RepresentationInfo,
    /// Optional audio representation (None if no audio AdaptationSet exists).
    pub audio: Option<RepresentationInfo>,
    /// MPD-level resolved BaseURL chain. Always non-empty: at minimum it
    /// contains the URL the MPD was fetched from.
    pub mpd_base_urls: Vec<Url>,
    /// Total number of `<Period>` elements in the source MPD (>=1).
    pub period_count: usize,
}

/// Resolved metadata for a single Representation chosen from an AdaptationSet.
#[derive(Debug)]
pub struct RepresentationInfo {
    /// `Representation@id` from the source MPD. Never empty (synthesised if
    /// the source omitted it).
    pub id: String,
    /// Declared average bandwidth in bits per second; `0` when missing.
    pub bandwidth: u64,
    /// RFC 6381 codec string, e.g. `"avc1.640028"`. Inherited from
    /// AdaptationSet when not present on the Representation.
    pub codecs: Option<String>,
    /// MIME type, e.g. `"video/mp4"`. Inherited from AdaptationSet when not
    /// present on the Representation.
    pub mime_type: Option<String>,
    /// Declared frame width in pixels.
    pub width: Option<u32>,
    /// Declared frame height in pixels.
    pub height: Option<u32>,
    /// RFC 5646 language tag for the AdaptationSet.
    pub lang: Option<String>,
    /// Resolved BaseURL chain for this Representation:
    /// MPD BaseURL ▸ Period BaseURL ▸ AdaptationSet BaseURL ▸ Representation BaseURL.
    /// Always non-empty.
    pub base_urls: Vec<Url>,
    // Task 3 will add: pub plan: SegmentPlan,
}

/// Parse an MPD body and select the highest-bandwidth video representation
/// (and matching audio, when any) from the first period.
///
/// `base_url` is the URL the MPD itself was fetched from; it acts as the
/// root for resolving relative `<BaseURL>` elements.
///
/// # Errors
///
/// Returns [`DashError::Parse`] on malformed XML or missing `<Period>`,
/// [`DashError::DynamicMpd`] for live MPDs, [`DashError::DrmProtected`] when
/// any `ContentProtection` element is present, and
/// [`DashError::NoVideoRepresentation`] when no video AdaptationSet is found.
pub fn parse_mpd(body: &str, base_url: &Url) -> Result<ParsedManifest, DashError> {
    let mpd = dash_mpd::parse(body).map_err(|e| DashError::Parse(e.to_string()))?;

    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(DashError::DynamicMpd);
    }

    // DRM gate. Refuse before representation selection so DRM beats
    // "no video repr" in error precedence.
    if mpd_has_drm(&mpd) {
        return Err(DashError::DrmProtected);
    }

    let period_count = mpd.periods.len();
    if period_count > 1 {
        log::warn!(
            "MPD has {period_count} periods; only the first is consumed (multi-period support deferred)"
        );
    }

    let period = mpd
        .periods
        .first()
        .ok_or_else(|| DashError::Parse("no <Period>".into()))?;

    let mpd_base_urls = resolve_base_urls(&mpd.base_url, base_url);
    let period_base_urls = resolve_period_base_urls(period, &mpd_base_urls);

    let period_duration = period
        .duration
        .or(mpd.mediaPresentationDuration)
        .unwrap_or_default();

    let video = pick_video(period, &period_base_urls)?;
    let audio = match pick_audio(period, &period_base_urls) {
        Ok(info) => Some(info),
        Err(DashError::NoAudioRepresentation) => None,
        Err(e) => return Err(e),
    };

    Ok(ParsedManifest {
        period_duration,
        video,
        audio,
        mpd_base_urls,
        period_count,
    })
}

/// Returns true if any `ContentProtection` element appears at MPD, Period,
/// AdaptationSet, or Representation scope.
fn mpd_has_drm(mpd: &dash_mpd::MPD) -> bool {
    if !mpd.ContentProtection.is_empty() {
        return true;
    }
    for period in &mpd.periods {
        if !period.ContentProtection.is_empty() {
            return true;
        }
        for aset in &period.adaptations {
            if !aset.ContentProtection.is_empty() {
                return true;
            }
            for repr in &aset.representations {
                if !repr.ContentProtection.is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

/// Resolve a list of `<BaseURL>` elements against a parent URL. When the
/// list is empty, returns `[parent]` so the chain is always non-empty.
/// Entries that fail to parse against the parent are dropped with a warning.
fn resolve_base_urls(base_urls: &[dash_mpd::BaseURL], parent: &Url) -> Vec<Url> {
    if base_urls.is_empty() {
        return vec![parent.clone()];
    }
    let mut out = Vec::with_capacity(base_urls.len());
    for b in base_urls {
        match parent.join(&b.base) {
            Ok(u) => out.push(u),
            Err(e) => log::warn!("failed to resolve BaseURL {:?}: {e}", b.base),
        }
    }
    if out.is_empty() {
        // All entries failed — fall back to parent so the chain stays non-empty.
        out.push(parent.clone());
    }
    out
}

/// Build the Period-scoped BaseURL chain by resolving Period BaseURLs
/// against the MPD-level chain (using the first MPD BaseURL as the parent).
fn resolve_period_base_urls(period: &dash_mpd::Period, mpd_base_urls: &[Url]) -> Vec<Url> {
    // resolve_base_urls always returns a non-empty Vec, so .first() should
    // not fail here; the unwrap_or_else is defensive only.
    let parent = mpd_base_urls.first().cloned().unwrap_or_else(|| {
        #[allow(clippy::unwrap_used)]
        Url::parse("about:blank").unwrap()
    });
    resolve_base_urls(&period.BaseURL, &parent)
}

/// Build the AdaptationSet-scoped BaseURL chain by resolving the AS's
/// BaseURLs against the period chain.
fn resolve_aset_base_urls(aset: &dash_mpd::AdaptationSet, period_base_urls: &[Url]) -> Vec<Url> {
    let parent = period_base_urls.first().cloned().unwrap_or_else(|| {
        #[allow(clippy::unwrap_used)]
        Url::parse("about:blank").unwrap()
    });
    resolve_base_urls(&aset.BaseURL, &parent)
}

/// Build the Representation-scoped BaseURL chain by resolving the
/// Representation's BaseURLs against the AdaptationSet chain.
fn resolve_repr_base_urls(repr: &dash_mpd::Representation, aset_base_urls: &[Url]) -> Vec<Url> {
    let parent = aset_base_urls.first().cloned().unwrap_or_else(|| {
        #[allow(clippy::unwrap_used)]
        Url::parse("about:blank").unwrap()
    });
    resolve_base_urls(&repr.BaseURL, &parent)
}

/// Returns true if the AdaptationSet describes video content. Trusts
/// `@contentType` first, falls back to `@mimeType` prefix.
fn is_video_aset(aset: &dash_mpd::AdaptationSet) -> bool {
    if let Some(ct) = aset.contentType.as_deref() {
        return ct.eq_ignore_ascii_case("video");
    }
    aset.mimeType
        .as_deref()
        .is_some_and(|m| m.to_ascii_lowercase().starts_with("video/"))
}

/// Returns true if the AdaptationSet describes audio content. Trusts
/// `@contentType` first, falls back to `@mimeType` prefix.
fn is_audio_aset(aset: &dash_mpd::AdaptationSet) -> bool {
    if let Some(ct) = aset.contentType.as_deref() {
        return ct.eq_ignore_ascii_case("audio");
    }
    aset.mimeType
        .as_deref()
        .is_some_and(|m| m.to_ascii_lowercase().starts_with("audio/"))
}

/// Pick the highest-bandwidth video Representation across all video
/// AdaptationSets in the period.
fn pick_video(
    period: &dash_mpd::Period,
    period_base_urls: &[Url],
) -> Result<RepresentationInfo, DashError> {
    pick_representation(period, period_base_urls, is_video_aset)
        .ok_or(DashError::NoVideoRepresentation)
}

/// Pick the highest-bandwidth audio Representation across all audio
/// AdaptationSets in the period.
fn pick_audio(
    period: &dash_mpd::Period,
    period_base_urls: &[Url],
) -> Result<RepresentationInfo, DashError> {
    pick_representation(period, period_base_urls, is_audio_aset)
        .ok_or(DashError::NoAudioRepresentation)
}

/// Generic Representation picker: filter AdaptationSets by `match_aset`,
/// then pick the highest-bandwidth Representation from the union.
fn pick_representation(
    period: &dash_mpd::Period,
    period_base_urls: &[Url],
    match_aset: fn(&dash_mpd::AdaptationSet) -> bool,
) -> Option<RepresentationInfo> {
    let mut best: Option<(u64, &dash_mpd::AdaptationSet, &dash_mpd::Representation)> = None;

    for aset in &period.adaptations {
        if !match_aset(aset) {
            continue;
        }
        for repr in &aset.representations {
            let bw = repr.bandwidth.unwrap_or(0);
            match best {
                Some((b, _, _)) if b >= bw => {}
                _ => best = Some((bw, aset, repr)),
            }
        }
    }

    let (bandwidth, aset, repr) = best?;
    let aset_base_urls = resolve_aset_base_urls(aset, period_base_urls);
    let base_urls = resolve_repr_base_urls(repr, &aset_base_urls);

    let id = repr
        .id
        .clone()
        .unwrap_or_else(|| format!("repr-{bandwidth}"));

    let codecs = repr.codecs.clone().or_else(|| aset.codecs.clone());
    let mime_type = repr.mimeType.clone().or_else(|| aset.mimeType.clone());

    Some(RepresentationInfo {
        id,
        bandwidth,
        codecs,
        mime_type,
        width: repr
            .width
            .or(aset.width)
            .and_then(|w| u32::try_from(w).ok()),
        height: repr
            .height
            .or(aset.height)
            .and_then(|h| u32::try_from(h).ok()),
        lang: aset.lang.clone(),
        base_urls,
    })
}
