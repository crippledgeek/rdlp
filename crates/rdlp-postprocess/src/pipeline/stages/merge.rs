//! `MergeStage` — merges separate video and audio streams into a single file.
//!
//! This stage runs first (index 0) when there are 2+ current files.
//! Uses `rdlp_ffmpeg::FFmpegRunner::merge()` via stream copy (no re-encoding).
//!
//! # Lint allowances
//!
//! - `clippy::indexing_slicing`: `current_files[0]` and `current_files[1]`
//!   are accessed only after `should_run()` checks `len() >= 2`.

#![allow(clippy::indexing_slicing)]

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info};

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};

use rdlp_types::ContainerFormat;

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Merges separate video + audio streams into a single container.
///
/// `should_run` triggers when `tracker.current_files.len() >= 2`.
/// Uses `FileTracker::temp_path` for output — no naming collisions.
pub struct MergeStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl MergeStage {
    /// Create a new `MergeStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Determine the output container from config, input extensions, and
    /// (when available) the source `Format`s' declared codecs.
    ///
    /// The codec-aware path mirrors yt-dlp's `get_compatible_ext`
    /// (`yt_dlp/utils/_utils.py`) so that VP9 video paired with AAC audio
    /// — extension-compatible with `.mp4` but codec-incompatible — gets
    /// routed to MKV instead of producing a VP9-in-MP4 file that some
    /// players (notably older VLC versions) refuse to play.
    ///
    /// `video_codec` / `audio_codec` come from the source `Format` records
    /// (forwarded via `info.requested_formats` set by the orchestrator's
    /// merge dispatch). When either is `None`, falls back to the
    /// extension-only heuristic.
    fn determine_output_format(
        config: &rdlp_types::PostProcess,
        video_ext: Option<&str>,
        audio_ext: Option<&str>,
        video_codec: Option<&str>,
        audio_codec: Option<&str>,
    ) -> ContainerFormat {
        if let Some(format) = config.merge_output_format {
            return format;
        }
        // When both codecs are known, mirror yt-dlp's get_compatible_ext:
        // try mp4 → webm → mkv (mkv as the catch-all when neither container
        // fits the codec pair). This is the path that routes VP9+AAC to
        // mkv even though both source files have .mp4 / .m4a extensions.
        if let (Some(v), Some(a)) = (video_codec, audio_codec) {
            let container = Self::compatible_ext_from_codecs(v, a).unwrap_or(ContainerFormat::Mkv);
            debug!(
                vcodec = v, acodec = a, ext = container.as_ext();
                "MergeStage: codec-aware container picked"
            );
            return container;
        }
        // Ext-only fallback (codec info missing — e.g. plugin-emitted Format
        // with no vcodec/acodec). Conservative: webm ext → mkv (broadest
        // playability), else mp4.
        match (video_ext, audio_ext) {
            (Some("webm"), _) | (_, Some("webm")) => ContainerFormat::Mkv,
            _ => {
                debug!("No codec info available; defaulting to MP4 by ext heuristic");
                ContainerFormat::Mp4
            }
        }
    }

    /// Pick the most compatible container for the given codec pair.
    ///
    /// Ports yt-dlp's `get_compatible_ext` for the simple 1-video + 1-audio
    /// case. Codec strings are sanitized: lowercased, anything after the
    /// first `.` is dropped, and `0`s are stripped (matches yt-dlp's
    /// `try_get(getter=lambda x: x[0].split('.')[0].replace('0','').lower())`).
    /// This normalises `avc1.640028` → `avc1`, `vp09.00.30.08` → `vp9`,
    /// `mp4a.40.2` → `mp4a`, `av01.0.04M.08` → `av1`.
    ///
    /// Returns `Some(ext)` when both codecs fit one container; `None`
    /// signals "use MKV as the catch-all" (caller's responsibility, via
    /// fall-through to the extension-only path).
    fn compatible_ext_from_codecs(vcodec: &str, acodec: &str) -> Option<ContainerFormat> {
        // MP4-compatible: video + audio fourccs that ISO BMFF accepts.
        const MP4_COMPAT: &[&str] = &[
            "av1", "hevc", "avc1", "h264", "mp4a", "ac-4", "aacl", "ec-3",
        ];
        // WebM-compatible: VPx / AV1 video + Opus / Vorbis audio.
        const WEBM_COMPAT: &[&str] = &["av1", "vp9", "vp8", "opus", "vrbs"];

        let v = Self::sanitize_codec(vcodec);
        let a = Self::sanitize_codec(acodec);

        if MP4_COMPAT.contains(&v.as_str()) && MP4_COMPAT.contains(&a.as_str()) {
            return Some(ContainerFormat::Mp4);
        }
        if WEBM_COMPAT.contains(&v.as_str()) && WEBM_COMPAT.contains(&a.as_str()) {
            return Some(ContainerFormat::WebM);
        }
        None
    }

    /// Build the mux options for a resolved output container.
    ///
    /// Exists as a named function so the faststart decision is observable in a
    /// test. Inlined into `process()` it was unreachable without a real `FFmpeg`
    /// run, which let #539 ship: a test could assert `supports_faststart()` on
    /// a container it supplied itself and never touch the production
    /// expression at all.
    fn merge_opts(output_format: ContainerFormat, encoding_tool: Option<String>) -> RemuxOptions {
        RemuxOptions {
            faststart: output_format.supports_faststart(),
            encoding_tool_override: encoding_tool,
            ..Default::default()
        }
    }

    /// Sanitize a codec string per yt-dlp's normalisation.
    ///
    /// `avc1.640028` → `avc1`; `vp09.00.30.08` → `vp9`; `mp4a.40.2` → `mp4a`;
    /// `av01.0.04M.08` → `av1`. `vorbis` stays `vorbis` (`replace('0','')`
    /// strips digit zeros only); yt-dlp's `COMPATIBLE_CODECS` uses `vrbs`
    /// as an alias — we accept either form.
    fn sanitize_codec(codec: &str) -> String {
        let head = codec.split('.').next().unwrap_or(codec);
        let stripped: String = head.chars().filter(|c| *c != '0').collect();
        stripped.to_lowercase()
    }
}

#[async_trait]
impl PipelineStage for MergeStage {
    fn name(&self) -> &'static str {
        "MergeStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.tracker.current_files.len() >= 2
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        let files = &msg.tracker.current_files;

        if files.len() < 2 {
            return Ok(msg);
        }

        info!(
            "MergeStage: merging {} streams into single file",
            files.len()
        );

        // Probe to determine which is video and which is audio.
        let (video_file, audio_file) = if files.len() == 2 {
            let info1 = self
                .ffmpeg
                .probe(&files[0])
                .await
                .context("merge stage: failed to probe first input file")?;
            let info2 = self
                .ffmpeg
                .probe(&files[1])
                .await
                .context("merge stage: failed to probe second input file")?;

            if info1.has_video && !info1.has_audio && info2.has_audio {
                (files[0].clone(), files[1].clone())
            } else if info2.has_video && !info2.has_audio && info1.has_audio {
                (files[1].clone(), files[0].clone())
            } else if info1.has_video {
                (files[0].clone(), files[1].clone())
            } else {
                (files[1].clone(), files[0].clone())
            }
        } else {
            // More than 2 files — assume first is video, second is audio.
            (files[0].clone(), files[1].clone())
        };

        debug!(
            "MergeStage: video={}, audio={}",
            video_file.display(),
            audio_file.display()
        );

        let video_ext = video_file.extension().and_then(|e| e.to_str());
        let audio_ext = audio_file.extension().and_then(|e| e.to_str());

        // Pull declared codecs from the source Formats (orchestrator sets
        // `info.requested_formats = [video, audio]` for merge dispatch).
        // First Format with non-empty vcodec/acodec wins respectively, so
        // we don't depend on a particular ordering of the requested pair.
        let (video_codec, audio_codec) =
            msg.info
                .requested_formats
                .as_ref()
                .map_or((None, None), |formats| {
                    let vc = formats
                        .iter()
                        .find_map(|f| f.vcodec.as_str().filter(|s| !s.is_empty()));
                    let ac = formats
                        .iter()
                        .find_map(|f| f.acodec.as_str().filter(|s| !s.is_empty()));
                    (vc, ac)
                });

        let output_format = Self::determine_output_format(
            &msg.config,
            video_ext,
            audio_ext,
            video_codec,
            audio_codec,
        );

        // Use tracker.temp_path — no naming collision possible.
        let output_path = msg.tracker.temp_path(&video_file, output_format.as_ext());

        let opts = Self::merge_opts(output_format, msg.encoding_tool.clone());

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());
        let callback = stage_callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
            Arc::new(move |frac| cb.on_progress(rdlp_types::Progress::from_f64(frac)))
        });

        self.ffmpeg
            .merge(
                &video_file,
                &audio_file,
                &output_path,
                &opts,
                callback,
                Some(msg.cancel.clone()),
            )
            .await
            .context("merge stage failed")?;

        info!("MergeStage: merged to {}", output_path.display());

        // Promote output — input files become temps.
        msg.tracker.replace(vec![output_path]);

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_types::InfoDict;
    use rdlp_types::PostProcess;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test".to_string(),
                "Test".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(files, reg),
            config: Arc::new(PostProcess::default()),
            original_stem: "test".to_string(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn should_run_requires_two_files() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MergeStage::new(ffmpeg);

        let msg_one = make_msg(vec![PathBuf::from("/tmp/video.mp4")]);
        assert!(!stage.should_run(&msg_one));

        let msg_two = make_msg(vec![
            PathBuf::from("/tmp/video.mp4"),
            PathBuf::from("/tmp/audio.m4a"),
        ]);
        assert!(stage.should_run(&msg_two));
    }

    #[test]
    fn should_not_run_with_zero_files() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MergeStage::new(ffmpeg);
        let msg = make_msg(vec![]);
        assert!(!stage.should_run(&msg));
    }

    /// #539, fourth site: merge decides faststart from the container type.
    ///
    /// The issue named three sites; this one was missed. It is reachable
    /// whenever `merge_output_format` is an ISOBMFF container beyond mp4/mov,
    /// which the old `matches!(output_format, "mp4" | "mov")` excluded.
    #[test]
    fn merge_faststart_follows_the_container_type() {
        for (container, want) in [
            (ContainerFormat::Mp4, true),
            (ContainerFormat::Mov, true),
            (ContainerFormat::M4v, true),
            (ContainerFormat::F4v, true),
            (ContainerFormat::Mkv, false),
            (ContainerFormat::WebM, false),
        ] {
            let config = PostProcess {
                merge_output_format: Some(container),
                ..PostProcess::default()
            };
            let resolved =
                MergeStage::determine_output_format(&config, Some("mp4"), Some("m4a"), None, None);
            assert_eq!(resolved, container);
            // Assert on the options the stage actually builds — asserting
            // `resolved.supports_faststart()` here would only re-test the
            // predicate with a value this test supplied, and would stay green
            // if the stage hardcoded `faststart: false`.
            assert_eq!(
                MergeStage::merge_opts(resolved, None).faststart,
                want,
                "merge faststart for {container:?} must follow supports_faststart()"
            );
        }
    }

    #[test]
    fn determine_output_format_explicit_config() {
        let config = PostProcess {
            merge_output_format: Some(rdlp_types::ContainerFormat::Mkv),
            ..PostProcess::default()
        };
        assert_eq!(
            MergeStage::determine_output_format(&config, Some("mp4"), Some("m4a"), None, None),
            ContainerFormat::Mkv
        );
    }

    #[test]
    fn determine_output_format_webm_ext_fallback_picks_mkv_when_no_codecs() {
        // Codec-unaware path: webm extension with no codec info → fallback to mkv.
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(&config, Some("webm"), Some("opus"), None, None),
            ContainerFormat::Mkv
        );
    }

    #[test]
    fn determine_output_format_default_mp4() {
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(&config, Some("mp4"), Some("m4a"), None, None),
            ContainerFormat::Mp4
        );
    }

    // ── Codec-aware container picker (closes #241 part 2/3) ──────────────

    #[test]
    fn determine_output_format_h264_aac_picks_mp4() {
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(
                &config,
                Some("mp4"),
                Some("m4a"),
                Some("avc1.640028"),
                Some("mp4a.40.2"),
            ),
            ContainerFormat::Mp4
        );
    }

    #[test]
    fn determine_output_format_vp9_aac_picks_mkv() {
        // VP9 video paired with AAC audio — neither codec fits both mp4
        // (vp9 not in mp4 set) nor webm (aac not in webm set). yt-dlp's
        // get_compatible_ext final fallback is mkv; rdlp mirrors that.
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(
                &config,
                Some("mp4"),
                Some("m4a"),
                Some("vp09.00.30.08"),
                Some("mp4a.40.2"),
            ),
            ContainerFormat::Mkv
        );
    }

    #[test]
    fn compatible_ext_from_codecs_vp9_aac_returns_none() {
        // Direct test of the codec-only matcher: VP9 + AAC is not
        // compatible with either mp4 or webm → None (caller falls through
        // to ext-only path, ultimately mkv via the final fallback).
        assert_eq!(
            MergeStage::compatible_ext_from_codecs("vp09.00.30.08", "mp4a.40.2"),
            None,
            "VP9+AAC must not co-fit mp4 (vp9 not in mp4 set) or webm (aac not in webm set)"
        );
    }

    #[test]
    fn determine_output_format_av1_opus_picks_webm() {
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(
                &config,
                Some("webm"),
                Some("opus"),
                Some("av01.0.04M.08"),
                Some("opus"),
            ),
            ContainerFormat::WebM
        );
    }

    #[test]
    fn determine_output_format_vp9_opus_picks_webm() {
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(
                &config,
                Some("webm"),
                Some("opus"),
                Some("vp09.00.30.08"),
                Some("opus"),
            ),
            ContainerFormat::WebM
        );
    }

    #[test]
    fn determine_output_format_hevc_aac_picks_mp4() {
        let config = PostProcess::default();
        assert_eq!(
            MergeStage::determine_output_format(
                &config,
                Some("mp4"),
                Some("m4a"),
                Some("hevc"),
                Some("mp4a"),
            ),
            ContainerFormat::Mp4
        );
    }

    #[test]
    fn sanitize_codec_strips_dots_and_zeros() {
        assert_eq!(MergeStage::sanitize_codec("avc1.640028"), "avc1");
        assert_eq!(MergeStage::sanitize_codec("vp09.00.30.08"), "vp9");
        assert_eq!(MergeStage::sanitize_codec("mp4a.40.2"), "mp4a");
        assert_eq!(MergeStage::sanitize_codec("av01.0.04M.08"), "av1");
        assert_eq!(MergeStage::sanitize_codec("OPUS"), "opus");
    }

    #[test]
    fn sanitize_codec_handles_empty_and_unknown() {
        // Defensive: empty string and unrecognised codec strings must not
        // panic; they fall through to the `None` branch in
        // `compatible_ext_from_codecs` and the caller routes to mkv.
        assert_eq!(MergeStage::sanitize_codec(""), "");
        assert_eq!(MergeStage::sanitize_codec("xyzzy"), "xyzzy");
        assert_eq!(
            MergeStage::compatible_ext_from_codecs("", ""),
            None,
            "empty codec strings must not match any container"
        );
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MergeStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }
}
