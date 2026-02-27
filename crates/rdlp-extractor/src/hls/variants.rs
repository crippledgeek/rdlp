//! HLS metadata detection and variant expansion
//!
//! Comprehensive metadata extraction from M3U8 playlists and
//! expansion of master playlists into per-variant format entries.

use super::detector::HlsSizeDetector;
use super::types::{HlsInfo, HlsVariantInfo};
use crate::base::common::BaseExtractor;
use log::debug;
use rdlp_core::{RdlpError, Result};

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
    audio_codec.or(
        if video_codec.is_some() && !has_separate_audio_group {
            Some("aac")
        } else {
            None
        },
    )
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
    pub async fn detect_hls_metadata(&self, m3u8_url: &str) -> Result<Option<HlsInfo>> {
        BaseExtractor::validate_url_security(m3u8_url)?;

        if self.verbose {
            debug!(url:? = m3u8_url; "HLS detecting metadata");
        }

        let playlist_text = match self.fetch_playlist_text(m3u8_url).await {
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
                let base_url = url::Url::parse(m3u8_url)
                    .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;
                let media_url = base_url
                    .join(&variant.uri)
                    .map_err(|e| {
                        RdlpError::Extraction(format!("Failed to join media playlist URL: {e}"))
                    })?
                    .to_string();

                BaseExtractor::validate_url_security(&media_url)?;

                let media_info = match self.fetch_and_extract_media_info(&media_url).await {
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
    pub async fn detect_hls_variants(&self, m3u8_url: &str) -> Result<Vec<HlsVariantInfo>> {
        BaseExtractor::validate_url_security(m3u8_url)?;

        let playlist_text = match self.fetch_playlist_text(m3u8_url).await {
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

        let base_url = url::Url::parse(m3u8_url)
            .map_err(|e| RdlpError::Extraction(format!("Invalid base URL: {e}")))?;

        // Resolve all variant media playlist URLs and extract per-variant metadata.
        // Skip I-frame-only variants (EXT-X-I-FRAME-STREAM-INF) — these are trick-play
        // playlists whose segments are not downloadable as standalone media.
        let mut variants: Vec<HlsVariantInfo> = Vec::with_capacity(master.variants.len());
        for variant in master.variants.iter().filter(|v| !v.is_i_frame) {
            let media_url = match base_url.join(&variant.uri) {
                Ok(u) => u.to_string(),
                Err(_) => continue,
            };
            let (video_codec, audio_codec) = variant
                .codecs
                .as_deref()
                .map(rdlp_core::parse_hls_codecs)
                .unwrap_or((None, None));
            let audio_codec =
                infer_muxed_audio(video_codec, audio_codec, variant.audio.is_some());

            variants.push(HlsVariantInfo {
                media_playlist_url: media_url,
                resolution: variant.resolution.as_ref().map(|r| (r.width, r.height)),
                video_codec: video_codec.map(String::from),
                audio_codec: audio_codec.map(String::from),
                frame_rate: variant.frame_rate,
                bandwidth: variant.bandwidth,
                average_bandwidth: variant.average_bandwidth,
                // Shared fields filled below
                segment_count: 0,
                total_duration: None,
                is_live: false,
                has_encryption: false,
                segment_container: None,
            });
        }

        // Fetch shared media info from the best non-I-frame variant (highest bandwidth)
        let best_variant = master
            .variants
            .iter()
            .filter(|v| !v.is_i_frame)
            .max_by_key(|v| v.bandwidth)
            .expect("master playlist has at least one non-I-frame variant");
        let best_media_url = base_url
            .join(&best_variant.uri)
            .map_err(|e| RdlpError::Extraction(format!("Failed to join media URL: {e}")))?
            .to_string();

        if let Ok(Some(media_info)) = self.fetch_and_extract_media_info(&best_media_url).await {
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
