//! Video conversion: remux and transcoding.
//!
//! Provides `convert_video` (async entry point) plus synchronous helpers for
//! video transcoding with filter graph pixel format conversion, and video
//! encoder packet writing.
//!
//! The 6 transcode phases live in `video_transcode_phases.rs`; the phase
//! state structs (`Phase1Outputs`, `AudioTranscodeState`,
//! `VideoTranscodeContext`) live in `video_transcode_context.rs`.

use std::path::Path;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::error::{PostProcessError, Result};

use super::super::salvage::prepare_input_with_salvage;
use super::super::{FFmpegRunner, RemuxOptions, VideoConvertOptions, ensure_init};

/// Callback type for forwarding `FFmpeg` log lines to the UI.
type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

impl FFmpegRunner {
    /// Convert a video file, either by remuxing or transcoding.
    ///
    /// Uses `opts.remux_only` to determine whether to stream-copy or transcode.
    /// For transcoding, encodes video with the specified codec while optionally
    /// copying the audio stream unchanged.
    ///
    /// Automatically detects and salvages corrupt Matroska/WebM containers
    /// before conversion to prevent EBML-induced muxer failures.
    ///
    /// # Errors
    ///
    /// Returns an error if probing, decoding, encoding, or muxing fails —
    /// including I/O errors, unsupported codec errors, and ENOMEM during
    /// mux write.
    pub async fn convert_video(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &VideoConvertOptions,
        progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>>,
        log_fn: Option<LogFn>,
        cancel: Option<CancellationToken>,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("convert_video", move || {
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, true)?;

            // Capture FFmpeg C-level logs when verbose mode is enabled.
            let log_guard = if opts.verbose {
                super::super::log_capture::LogCaptureGuard::begin().ok()
            } else {
                None
            };

            let result = Self::convert_video_sync(
                &effective_input,
                &output,
                &opts,
                progress_fn.as_deref(),
                cancel.as_ref(),
            );

            // Drain captured logs and forward to the UI log viewer.
            if let Some(ref guard) = log_guard
                && let Ok(lines) = guard.take_captured()
                && let Some(ref log) = log_fn
            {
                for line in lines {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        log(trimmed);
                    }
                }
            }

            if let Some(ref temp) = salvage_temp {
                // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
                #[allow(clippy::disallowed_methods)]
                let _ = std::fs::remove_file(temp);
            }

            Ok(result?)
        })
        .await
    }

    /// Convert video synchronously (dispatches to remux or transcode).
    fn convert_video_sync(
        input: &Path,
        output: &Path,
        opts: &VideoConvertOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
        if opts.remux_only {
            // Determine if output is MP4/MOV for faststart
            let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("");
            let remux_opts = RemuxOptions {
                faststart: ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("mov"),
                ..Default::default()
            };
            Ok(Self::remux_sync(input, output, &remux_opts, progress_fn)
                .map_err(|e| PostProcessError::ffmpeg_failed(format!("{e:#}")))?)
        } else {
            Self::convert_video_transcode_sync(input, output, opts, progress_fn, cancel)
        }
    }

    /// Transcode video to a target codec, optionally copying audio.
    ///
    /// Decodes video frames, converts pixel format through a filter graph,
    /// and encodes with the target video codec. Audio is stream-copied if
    /// `opts.audio_copy` is true.
    fn convert_video_transcode_sync(
        input: &Path,
        output: &Path,
        opts: &VideoConvertOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
        ensure_init()?;
        let phase1 = Self::open_input_and_decoder(input)?;
        let mut ctx = Self::configure_video_encoder(phase1, opts, output)?;
        Self::setup_audio_pipeline(&mut ctx)?;
        Self::write_header_and_build_filter(&mut ctx)?;
        Self::run_encode_loop(&mut ctx, progress_fn, cancel)?;
        Self::finalize_transcode(ctx)
    }
}
