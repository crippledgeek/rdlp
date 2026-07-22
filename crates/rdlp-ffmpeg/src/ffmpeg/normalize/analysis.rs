//! Audio analysis functions for normalization.
//!
//! Contains the shared analysis decode loop used by both peak analysis
//! (`astats` filter) and loudnorm pass 1 (`loudnorm` filter), plus
//! metadata helpers and loudness verification.

use std::ffi::CStr;
use std::path::Path;

use anyhow::Context as _;
use log::{debug, warn};
use tokio_util::sync::CancellationToken;

use crate::error::{FfmpegResultExt as _, PostProcessError, Result};

use super::super::ffi_helpers::filter_graph::{AudioSinkSpec, build_audio_filter_graph};
use super::super::ffi_helpers::{frame_unref_audio, set_single_thread_codec};
use super::super::log_capture::LogSuppressGuard;
use super::super::{FFmpegRunner, NormalizeOptions, PeakAnalysis, ensure_init};
use super::helpers::{begin_loudnorm_capture, parse_loudnorm_json};

use crate::ffmpeg::LoudnormMeasurements;

/// Run the shared analysis decode loop: open → decode → filter → flush.
///
/// Opens `input`, finds the best audio stream, creates a single-threaded
/// decoder, builds a filter graph with `filter_spec`, then runs the full
/// decode → filter → drain loop. After each batch of filtered frames,
/// `on_drain` is called to process them (e.g. extract metadata or discard).
///
/// Both `analyze_peak_sync` and `loudnorm_pass1_sync` share this pipeline;
/// only the filter spec and frame handling differ.
pub(super) fn run_analysis_decode_loop(
    input: &Path,
    filter_spec: &str,
    label: &str,
    cancel: Option<&CancellationToken>,
    on_drain: &mut dyn FnMut(
        &mut ffmpeg_the_third::filter::Graph,
        &mut ffmpeg_the_third::frame::Audio,
    ) -> Result<()>,
) -> anyhow::Result<()> {
    ensure_init()?;

    let mut ictx = ffmpeg_the_third::format::input(input)
        .map_err(PostProcessError::from)
        .with_context(|| format!("failed to open input for analysis {}", input.display()))?;

    let ist_index = ictx
        .streams()
        .best(ffmpeg_the_third::media::Type::Audio)
        .map(|s| s.index())
        .ok_or(PostProcessError::NoAudioStream)?;

    let ist = ictx.stream(ist_index).ok_or_else(|| {
        PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
    })?;
    let ist_time_base = ist.time_base();

    let mut decoder_ctx =
        ffmpeg_the_third::codec::context::Context::from_parameters(ist.parameters())
            .ff_context("failed to create decoder context for analysis")?;
    set_single_thread_codec(unsafe { decoder_ctx.as_mut_ptr() });
    let mut decoder = decoder_ctx
        .decoder()
        .audio()
        .ff_context("failed to open audio decoder for analysis")?;

    debug!(
        "[{label}] decoder: rate={}, fmt={}, ch_layout={}, time_base={}/{}",
        decoder.rate(),
        decoder.format().name(),
        decoder.ch_layout().description(),
        ist_time_base.numerator(),
        ist_time_base.denominator(),
    );

    // Build filter graph (reuse shared helper). Measurement-only: the frames
    // are read by loudnorm and never handed to an encoder, so there is no
    // fixed frame size to honour.
    let mut graph = build_audio_filter_graph(
        &decoder,
        ist_time_base,
        AudioSinkSpec {
            filter_spec,
            frame_size: 0,
        },
    )?;

    // Skip non-audio streams to avoid allocating memory for large video packets
    FFmpegRunner::discard_non_audio_streams(&mut ictx, ist_index);

    let mut frame = ffmpeg_the_third::frame::Audio::empty();
    let mut filtered = ffmpeg_the_third::frame::Audio::empty();
    let mut packets_skipped = 0u64;

    // Suppress FFmpeg's C-level decoder error spam during decode loop —
    // we handle errors at the Rust level with rate-limited warnings.
    let _log_suppress = LogSuppressGuard::new();

    for result in ictx.packets() {
        crate::ffmpeg::transcode::check_cancelled(cancel)?;
        let (stream, packet) = result.ff_context("failed to read packet during analysis")?;
        if stream.index() != ist_index {
            continue;
        }
        if let Err(e) = decoder.send_packet(&packet) {
            if packets_skipped == 0 {
                warn!("Audio decoder error during {label} (skipping affected packets): {e}");
            }
            packets_skipped += 1;
            while decoder.receive_frame(&mut frame).is_ok() {}
            continue;
        }
        while decoder.receive_frame(&mut frame).is_ok() {
            graph
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)
                .ff_context("filter source add frame failed")?;
            frame_unref_audio(&mut frame);

            on_drain(&mut graph, &mut filtered)?;
        }
    }

    if packets_skipped > 0 {
        warn!("Skipped {packets_skipped} audio packet(s) during {label} due to decoder errors");
    }

    // Flush decoder — send_eof may fail if decoder is in a broken state
    if let Err(e) = decoder.send_eof() {
        warn!("Decoder send_eof failed during {label} (continuing with flush): {e}");
    }
    while decoder.receive_frame(&mut frame).is_ok() {
        graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .add(&frame)
            .ff_context("filter source add frame (flush) failed")?;
        frame_unref_audio(&mut frame);

        on_drain(&mut graph, &mut filtered)?;
    }

    // Flush filter
    graph
        .get("in")
        .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
        .source()
        .flush()
        .ff_context("filter source flush failed")?;
    on_drain(&mut graph, &mut filtered)?;

    Ok(())
}

/// Drain filtered frames from the graph "out" pad and update peak/RMS metadata.
///
/// Used by `analyze_peak_sync` to collect `Peak_level` and `RMS_level` from
/// the `astats` filter output.
fn drain_astats_metadata(
    graph: &mut ffmpeg_the_third::filter::Graph,
    filtered: &mut ffmpeg_the_third::frame::Audio,
    peak_db: &mut f64,
    rms_db: &mut f64,
) -> Result<()> {
    loop {
        let mut out_node = graph
            .get("out")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
        if out_node.sink().frame(filtered).is_err() {
            break;
        }
        if let Some(p) = read_frame_metadata(
            unsafe { filtered.as_ptr() },
            "lavfi.astats.Overall.Peak_level",
        ) {
            *peak_db = p;
        }
        if let Some(r) = read_frame_metadata(
            unsafe { filtered.as_ptr() },
            "lavfi.astats.Overall.RMS_level",
        ) {
            *rms_db = r;
        }
    }
    Ok(())
}

/// Drain filtered frames, discarding output. Used by loudnorm pass 1.
fn drain_discard(
    graph: &mut ffmpeg_the_third::filter::Graph,
    filtered: &mut ffmpeg_the_third::frame::Audio,
) -> Result<()> {
    loop {
        let mut out_node = graph
            .get("out")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
        if out_node.sink().frame(filtered).is_err() {
            break;
        }
        frame_unref_audio(filtered);
    }
    Ok(())
}

/// Read a metadata value from an `FFmpeg` frame as f64.
///
/// # Safety
///
/// `frame_ptr` must point to a valid `AVFrame`.
pub(super) fn read_frame_metadata(
    frame_ptr: *const ffmpeg_the_third::ffi::AVFrame,
    key: &str,
) -> Option<f64> {
    let key_cstr = std::ffi::CString::new(key).ok()?;
    unsafe {
        let metadata = (*frame_ptr).metadata;
        if metadata.is_null() {
            return None;
        }
        let entry =
            ffmpeg_the_third::ffi::av_dict_get(metadata, key_cstr.as_ptr(), std::ptr::null(), 0);
        if entry.is_null() {
            return None;
        }
        let value = CStr::from_ptr((*entry).value).to_string_lossy();
        if value.trim() == "-inf" {
            return Some(f64::NEG_INFINITY);
        }
        value.trim().parse::<f64>().ok()
    }
}

impl FFmpegRunner {
    /// Analyze peak and RMS levels using `astats` filter with frame metadata.
    pub(super) fn analyze_peak_sync(
        input: &Path,
        target_peak_db: f64,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<PeakAnalysis> {
        let mut peak_db = f64::NEG_INFINITY;
        let mut rms_db = f64::NEG_INFINITY;

        let astats_spec = "astats=metadata=1:reset=0:measure_perchannel=none:\
                           measure_overall=Peak_level+RMS_level";

        run_analysis_decode_loop(
            input,
            astats_spec,
            "peak analysis",
            cancel,
            &mut |graph, filtered| {
                drain_astats_metadata(graph, filtered, &mut peak_db, &mut rms_db)
            },
        )?;

        if peak_db == f64::NEG_INFINITY {
            return Err(PostProcessError::NormalizationFailed {
                message: "could not determine peak level from astats metadata".into(),
            }
            .into());
        }

        let gain_db = target_peak_db - peak_db;

        Ok(PeakAnalysis {
            peak_db,
            rms_db,
            gain_db,
        })
    }

    /// Loudnorm pass 1: run loudnorm filter in analysis mode, capture JSON.
    pub(super) fn loudnorm_pass1_sync(
        input: &Path,
        opts: &NormalizeOptions,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<LoudnormMeasurements> {
        let guard = begin_loudnorm_capture()?;

        // loudnorm only supports AV_SAMPLE_FMT_DBL; explicitly convert from
        // decoder format (typically fltp) since FFmpeg 8.0's auto-conversion
        // during graph_config may fail with EINVAL.
        let loudnorm_spec = format!(
            "aformat=sample_fmts=dbl,loudnorm=I={:.1}:TP={:.1}:LRA={:.1}:print_format=json",
            opts.target_i, opts.target_tp, opts.target_lra,
        );

        // Run decode loop — graph is built and dropped inside
        run_analysis_decode_loop(
            input,
            &loudnorm_spec,
            "loudnorm analysis",
            cancel,
            &mut |graph, filtered| drain_discard(graph, filtered),
        )?;

        // Now capture the log output (JSON was emitted during graph drop
        // inside run_analysis_decode_loop when Graph went out of scope)
        let lines = guard.take_captured()?;
        drop(guard);

        debug!("Captured {} log lines from loudnorm pass 1", lines.len());

        Ok(parse_loudnorm_json(&lines)?)
    }

    /// Post-normalization loudness verification.
    ///
    /// Runs loudnorm pass 1 on the **output** file and compares measured
    /// levels against targets. Warns on significant deviations but does
    /// not fail — the output is already written.
    #[allow(clippy::unnecessary_wraps)] // callers use `?` for consistency with fallible callers
    pub(super) fn verify_loudness_sync(
        output: &Path,
        opts: &NormalizeOptions,
    ) -> anyhow::Result<()> {
        debug!("Loudness verification: analyzing output...");
        // Verification runs post-write on the finished output — never gated by
        // user cancel (the output already exists; pass `None`).
        match Self::loudnorm_pass1_sync(output, opts, None) {
            Ok(measured) => {
                debug!(
                    "Loudness verification: I={:.1} LUFS, TP={:.1} dBTP, LRA={:.1} LU",
                    measured.input_i, measured.input_tp, measured.input_lra
                );

                let i_delta = (measured.input_i - opts.target_i).abs();
                if i_delta > 2.0 {
                    warn!(
                        "Loudness verification: integrated loudness off by {i_delta:.1} LU \
                         (measured={:.1}, target={:.1})",
                        measured.input_i, opts.target_i
                    );
                }
                if measured.input_tp > opts.target_tp + 0.5 {
                    warn!(
                        "Loudness verification: true peak exceeds target \
                         (measured={:.1} dBTP, target={:.1} dBTP)",
                        measured.input_tp, opts.target_tp
                    );
                }
                Ok(())
            }
            Err(e) => {
                warn!("Loudness verification failed (non-fatal): {e}");
                Ok(())
            }
        }
    }
}
