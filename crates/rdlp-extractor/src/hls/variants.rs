//! HLS metadata detection and variant expansion
//!
//! Comprehensive metadata extraction from M3U8 playlists and
//! expansion of master playlists into per-variant format entries.

use super::detector::HlsSizeDetector;
use super::types::{HlsInfo, HlsVariantInfo};
use crate::base::common::BaseExtractor;
use log::debug;
use rdlp_core::{RdlpError, Result};

/// Expand a parsed HLS master playlist into per-variant `HlsVariantInfo`
/// entries. Pure function — does no I/O; safe to unit-test against any
/// `base_url` (including loopback) without hitting the SSRF gate.
///
/// Emits two kinds of entries:
/// 1. One per `EXT-X-STREAM-INF` (skipping I-frame trick-play variants)
///    as a video-or-muxed entry. When the variant references a separate
///    `AUDIO` rendition group, `audio_codec` stays `None` (video-only).
/// 2. One per `EXT-X-MEDIA TYPE=AUDIO` rendition group member that
///    carries a `URI`, as an audio-only entry. The `audio_codec` is
///    sourced from the first paired `EXT-X-STREAM-INF`'s `CODECS`
///    attribute (audio component), falling back to `"mp4a"` (AAC) when
///    no paired variant declares one.
///
/// Shared fields (`segment_count`, `total_duration`, etc.) are left at
/// their defaults; callers populate them from a probed media playlist.
pub(crate) fn expand_master_variants(
    master: &m3u8_rs::MasterPlaylist,
    base_url: &url::Url,
) -> Vec<HlsVariantInfo> {
    let mut variants: Vec<HlsVariantInfo> =
        Vec::with_capacity(master.variants.len() + master.alternatives.len());

    // Skip I-frame-only variants (EXT-X-I-FRAME-STREAM-INF) — these are
    // trick-play playlists whose segments are not downloadable as
    // standalone media.
    for variant in master.variants.iter().filter(|v| !v.is_i_frame) {
        let Ok(media_url) = base_url.join(&variant.uri) else {
            continue;
        };
        let (video_codec, audio_codec) = variant
            .codecs
            .as_deref()
            .map(rdlp_core::parse_hls_codecs)
            .unwrap_or((None, None));
        let audio_codec = infer_muxed_audio(video_codec, audio_codec, variant.audio.is_some());

        variants.push(HlsVariantInfo {
            media_playlist_url: media_url.to_string(),
            resolution: variant.resolution.as_ref().map(|r| (r.width, r.height)),
            video_codec: video_codec.map(String::from),
            audio_codec: audio_codec.map(String::from),
            frame_rate: variant.frame_rate,
            bandwidth: variant.bandwidth,
            average_bandwidth: variant.average_bandwidth,
            is_audio_only: false,
            language: None,
            audio_group_id: variant.audio.clone(),
            rendition_name: None,
            segment_count: 0,
            total_duration: None,
            is_live: false,
            has_encryption: false,
            segment_container: None,
        });
    }

    // Expand `EXT-X-MEDIA TYPE=AUDIO` rendition groups into audio-only
    // variant entries. Skips TYPE=SUBTITLES / CLOSED-CAPTIONS / VIDEO
    // (not audio) and entries without a URI (in-band renditions).
    for media in &master.alternatives {
        if media.media_type != m3u8_rs::AlternativeMediaType::Audio {
            continue;
        }
        let Some(uri) = media.uri.as_deref() else {
            continue;
        };
        let Ok(audio_playlist_url) = base_url.join(uri) else {
            continue;
        };

        let acodec = master
            .variants
            .iter()
            .filter(|v| v.audio.as_deref() == Some(media.group_id.as_str()))
            .find_map(|v| {
                let codecs = v.codecs.as_deref()?;
                let (_, a) = rdlp_core::parse_hls_codecs(codecs);
                a.map(String::from)
            })
            .unwrap_or_else(|| "mp4a".to_string());

        variants.push(HlsVariantInfo {
            media_playlist_url: audio_playlist_url.to_string(),
            resolution: None,
            video_codec: None,
            audio_codec: Some(acodec),
            frame_rate: None,
            // EXT-X-MEDIA entries don't carry BANDWIDTH. 0 is a neutral
            // placeholder; downstream size detection can estimate bitrate
            // from segment sizes if needed.
            bandwidth: 0,
            average_bandwidth: None,
            is_audio_only: true,
            language: media.language.clone(),
            audio_group_id: Some(media.group_id.clone()),
            rendition_name: Some(media.name.clone()),
            segment_count: 0,
            total_duration: None,
            is_live: false,
            has_encryption: false,
            segment_container: None,
        });
    }

    variants
}

/// Infer muxed audio codec for HLS variants.
///
/// Per the HLS spec, when a variant stream has a video codec declared in
/// `CODECS` but no audio codec, and doesn't reference a separate audio
/// rendition via the `AUDIO` attribute, audio is muxed in the transport
/// stream segments. Default to "aac" — the universal HLS muxed audio codec.
///
/// Returns the audio codec to use: either the declared one or the inferred one.
pub(crate) fn infer_muxed_audio<'a>(
    video_codec: Option<&'a str>,
    audio_codec: Option<&'a str>,
    has_separate_audio_group: bool,
) -> Option<&'a str> {
    audio_codec.or(if video_codec.is_some() && !has_separate_audio_group {
        Some("aac")
    } else {
        None
    })
}

impl HlsSizeDetector {
    /// Detect comprehensive HLS metadata from an M3U8 playlist
    ///
    /// Fetches and parses the M3U8 playlist, extracting all available metadata:
    /// - From master playlists: resolution, codecs, frame rate, bandwidth
    /// - From media playlists: segment count, total duration, live/VOD, encryption
    ///
    /// This is more comprehensive than `count_segments()` but does not fetch segment
    /// sizes (no HEAD requests). Performance is similar: 1-2 HTTP requests.
    ///
    /// # Arguments
    /// * `m3u8_url` - URL of the HLS m3u8 playlist
    ///
    /// # Returns
    /// * `Ok(Some(HlsInfo))` - Metadata extracted from the playlist
    /// * `Ok(None)` - Metadata could not be determined (non-fatal)
    /// * `Err(_)` - Fatal error (network failure, invalid playlist, etc.)
    ///
    /// # Errors
    ///
    /// Returns `Err` when:
    /// - URL security validation fails (SSRF protection) (`RdlpError::Security`)
    ///
    /// # Panics
    ///
    /// Panics if a master playlist has variants but none can be selected
    /// (programmer error — the filter + `or_else` fallback should always find one).
    pub async fn detect_hls_metadata(
        &self,
        m3u8_url: &str,
        timeout: std::time::Duration,
    ) -> Result<Option<HlsInfo>> {
        BaseExtractor::validate_url_security(m3u8_url)?;

        if self.verbose {
            debug!(url:? = m3u8_url; "HLS detecting metadata");
        }

        let playlist_text = match self.fetch_playlist_text(m3u8_url, timeout).await {
            Ok(text) => text,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to fetch playlist: {e}");
                }
                return Ok(None);
            }
        };

        let playlist = match m3u8_rs::parse_playlist_res(playlist_text.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to parse playlist: {e:?}");
                }
                return Ok(None);
            }
        };

        match playlist {
            m3u8_rs::Playlist::MasterPlaylist(master) => {
                if master.variants.is_empty() {
                    return Ok(None);
                }

                // Select the non-I-frame variant with the highest bandwidth (best quality)
                // INVARIANT: the `.or_else` fallback re-iterates all variants, so
                // `None` is only possible if `master.variants` is empty — but that
                // is guarded by the early-return check above.
                #[allow(clippy::expect_used)]
                let variant = master
                    .variants
                    .iter()
                    .filter(|v| !v.is_i_frame)
                    .max_by_key(|v| v.bandwidth)
                    .or_else(|| master.variants.iter().max_by_key(|v| v.bandwidth))
                    .expect("master playlist has at least one variant");

                // Extract variant metadata
                let resolution = variant.resolution.as_ref().map(|r| (r.width, r.height));
                let (video_codec, audio_codec) = variant
                    .codecs
                    .as_deref()
                    .map(rdlp_core::parse_hls_codecs)
                    .unwrap_or((None, None));
                let audio_codec =
                    infer_muxed_audio(video_codec, audio_codec, variant.audio.is_some());
                let frame_rate = variant.frame_rate;
                let bandwidth = Some(variant.bandwidth);
                let average_bandwidth = variant.average_bandwidth;

                if self.verbose {
                    debug!(
                        variants = master.variants.len(),
                        bandwidth = variant.bandwidth,
                        resolution:? = resolution,
                        codecs:? = variant.codecs,
                        frame_rate:? = frame_rate;
                        "HLS master playlist metadata extracted"
                    );
                }

                // Resolve and fetch the media playlist for segment info
                let base_url = url::Url::parse(m3u8_url).map_err(|e| RdlpError::Extraction {
                    message: format!("Invalid base URL: {e}"),
                    url: Some(m3u8_url.to_string()),
                })?;
                let media_url = base_url
                    .join(&variant.uri)
                    .map_err(|e| RdlpError::Extraction {
                        message: format!("Failed to join media playlist URL: {e}"),
                        url: Some(m3u8_url.to_string()),
                    })?
                    .to_string();

                BaseExtractor::validate_url_security(&media_url)?;

                let media_info = match self.fetch_and_extract_media_info(&media_url, timeout).await
                {
                    Ok(Some(info)) => info,
                    Ok(None) | Err(_) => {
                        // Return what we have from the master playlist
                        return Ok(Some(HlsInfo {
                            total_size: None,
                            segment_count: 0,
                            total_duration: None,
                            resolution,
                            video_codec: video_codec.map(String::from),
                            audio_codec: audio_codec.map(String::from),
                            frame_rate,
                            bandwidth,
                            average_bandwidth,
                            is_live: false,
                            has_encryption: false,
                            segment_container: None,
                        }));
                    }
                };

                Ok(Some(HlsInfo {
                    total_size: None,
                    segment_count: media_info.segment_count,
                    total_duration: Some(media_info.total_duration),
                    resolution,
                    video_codec: video_codec.map(String::from),
                    audio_codec: audio_codec.map(String::from),
                    frame_rate,
                    bandwidth,
                    average_bandwidth,
                    is_live: media_info.is_live,
                    has_encryption: media_info.has_encryption,
                    segment_container: media_info.segment_container,
                }))
            }
            m3u8_rs::Playlist::MediaPlaylist(media) => {
                let info = Self::extract_media_playlist_info(&media);

                if self.verbose {
                    debug!(
                        segments = info.segment_count,
                        duration = info.total_duration,
                        is_live = info.is_live,
                        encrypted = info.has_encryption;
                        "HLS media playlist metadata extracted"
                    );
                }

                Ok(Some(HlsInfo {
                    total_size: None,
                    segment_count: info.segment_count,
                    total_duration: Some(info.total_duration),
                    resolution: None,
                    video_codec: None,
                    audio_codec: None,
                    frame_rate: None,
                    bandwidth: None,
                    average_bandwidth: None,
                    is_live: info.is_live,
                    has_encryption: info.has_encryption,
                    segment_container: info.segment_container,
                }))
            }
        }
    }

    /// Detect all quality variants from an HLS master playlist.
    ///
    /// Returns one `HlsVariantInfo` per variant in a master playlist, each with
    /// a resolved media playlist URL and per-variant metadata. Shared metadata
    /// (segment count, duration, etc.) is fetched from the best variant's media
    /// playlist and applied to all entries.
    ///
    /// For media playlists (non-master), returns an empty Vec.
    ///
    /// # Errors
    ///
    /// Returns `Err` when:
    /// - URL security validation fails (SSRF protection) (`RdlpError::Security`)
    /// - Master playlist URL parsing fails (`RdlpError::Extraction`)
    /// - Media URL join fails (`RdlpError::Extraction`)
    ///
    /// # Panics
    ///
    /// Panics if a master playlist has non-I-frame variants but none can be selected
    /// (programmer error — the filter + `or_else` fallback should always find one).
    pub async fn detect_hls_variants(
        &self,
        m3u8_url: &str,
        timeout: std::time::Duration,
    ) -> Result<Vec<HlsVariantInfo>> {
        BaseExtractor::validate_url_security(m3u8_url)?;

        let playlist_text = match self.fetch_playlist_text(m3u8_url, timeout).await {
            Ok(text) => text,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to fetch playlist for variant expansion: {e}");
                }
                return Ok(Vec::new());
            }
        };

        let playlist = match m3u8_rs::parse_playlist_res(playlist_text.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                if self.verbose {
                    debug!("HLS failed to parse playlist for variant expansion: {e:?}");
                }
                return Ok(Vec::new());
            }
        };

        let master = match playlist {
            m3u8_rs::Playlist::MasterPlaylist(m) => m,
            m3u8_rs::Playlist::MediaPlaylist(_) => {
                // Not a master playlist — caller should fall back to detect_hls_metadata
                return Ok(Vec::new());
            }
        };

        if master.variants.is_empty() {
            return Ok(Vec::new());
        }

        let base_url = url::Url::parse(m3u8_url).map_err(|e| RdlpError::Extraction {
            message: format!("Invalid base URL: {e}"),
            url: Some(m3u8_url.to_string()),
        })?;

        let mut variants = expand_master_variants(&master, &base_url);
        if variants.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch shared media info from the best non-I-frame variant (highest bandwidth)
        // INVARIANT: `expand_master_variants` (called above) also filters to
        // non-I-frame variants and its output is non-empty (guarded at line 376),
        // so `master.variants` contains at least one non-I-frame entry here.
        #[allow(clippy::expect_used)]
        let best_variant = master
            .variants
            .iter()
            .filter(|v| !v.is_i_frame)
            .max_by_key(|v| v.bandwidth)
            .expect("master playlist has at least one non-I-frame variant");
        let best_media_url = base_url
            .join(&best_variant.uri)
            .map_err(|e| RdlpError::Extraction {
                message: format!("Failed to join media URL: {e}"),
                url: Some(m3u8_url.to_string()),
            })?
            .to_string();

        if let Ok(Some(media_info)) = self
            .fetch_and_extract_media_info(&best_media_url, timeout)
            .await
        {
            for v in &mut variants {
                v.segment_count = media_info.segment_count;
                v.total_duration = Some(media_info.total_duration);
                v.is_live = media_info.is_live;
                v.has_encryption = media_info.has_encryption;
                v.segment_container = media_info.segment_container.clone();
            }
        }

        if self.verbose {
            debug!(
                variants = variants.len(),
                url:? = m3u8_url;
                "HLS master playlist expanded into variants"
            );
        }

        Ok(variants)
    }
}
