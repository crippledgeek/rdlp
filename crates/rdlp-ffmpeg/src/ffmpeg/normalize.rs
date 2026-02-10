//! Audio normalization via FFmpeg library bindings.
//!
//! Two modes:
//! - **Peak**: Analyze peak/RMS levels via `astats` filter frame metadata,
//!   then apply `volume` + `alimiter` filters to normalize to a target peak.
//! - **Loudnorm**: EBU R128 two-pass normalization via `loudnorm` filter.
//!   Pass 1 captures measurements from FFmpeg log output, pass 2 applies
//!   them with `linear=true` for high-quality correction.

use std::ffi::CStr;
use std::path::Path;

use log::{debug, info, warn};

use crate::error::{PostProcessError, Result};

use super::ffi_helpers::{codec_threading_info, frame_unref_audio, set_single_thread_codec};
use super::log_capture::{LogCaptureGuard, LogSuppressGuard};
use super::salvage::{prepare_input_with_salvage, salvage_remux_sync};
use super::transcode::MuxTimingState;
use super::{
    AudioNormMode, FFmpegRunner, LoudnormMeasurements, NormalizeOptions, PeakAnalysis, ensure_init,
};

/// Extra headroom (dB) subtracted from the alimiter ceiling to account for
/// inter-sample true peak overshoot and lossy encoder artifacts.
///
/// `alimiter` is a sample-level limiter — it clamps digital sample values but
/// EBU R128 true peak measurement uses 4× oversampling to detect inter-sample
/// peaks.  Resampling (e.g. 44.1→48 kHz) and lossy encoding (AAC, Opus) can
/// also introduce ~0.5-2 dB of peak overshoot.  1.5 dB headroom is standard
/// broadcast practice (ITU-R BS.1770-5 recommendation).
const ALIMITER_TP_HEADROOM_DB: f64 = 1.5;

/// Build the alimiter filter spec with true-peak headroom.
///
/// Ceiling is `10^((target_tp - headroom) / 20)` in linear scale.
fn build_alimiter_spec(target_tp: f64) -> String {
    let ceiling = 10f64.powf((target_tp - ALIMITER_TP_HEADROOM_DB) / 20.0);
    format!("alimiter=limit={ceiling:.6}:attack=5:release=50")
}

/// Build the loudnorm pass 2 core filter string (without alimiter).
///
/// Returns the loudnorm filter (optionally preceded by acompressor when
/// `opts.precompress` is true).  The caller is responsible for appending
/// the alimiter via [`build_alimiter_spec`] at the correct position in
/// the filter chain:
///
/// - **Library path**: after `aresample` so resampling overshoot is caught.
/// - **CLI path**: after loudnorm (FFmpeg CLI inserts its own converters).
///
/// Default strategy: always `linear=true`.  FFmpeg's loudnorm with
/// `linear=true` falls back to dynamic internally when conditions aren't
/// met, so forcing `linear=false` is unnecessary and often produces worse
/// perceived loudness due to over-compression.
///
/// When `opts.force_dynamic` is true, uses `linear=false` instead.
fn build_loudnorm_pass2_filter(
    opts: &NormalizeOptions,
    measurements: &LoudnormMeasurements,
) -> String {
    let shortfall = measurements.linear_shortfall(opts.target_i, opts.target_tp);
    let predicted_gain = measurements.predict_linear_gain(opts.target_i, opts.target_tp);

    info!(
        "Loudnorm analysis: desired_gain={:.1} dB, predicted_linear_gain={:.1} dB, \
         shortfall={:.1} LU",
        opts.target_i - measurements.input_i,
        predicted_gain,
        shortfall,
    );

    let linear_mode = if opts.force_dynamic {
        info!("Strategy: dynamic (forced via --loudnorm-dynamic)");
        "false"
    } else {
        info!(
            "Strategy: linear (shortfall={shortfall:.1} LU, \
             loudnorm handles internal fallback to dynamic if needed)"
        );
        "true"
    };

    let m = measurements;
    let loudnorm = format!(
        "loudnorm=I={:.1}:TP={:.1}:LRA={:.1}:measured_I={:.2}:measured_TP={:.2}:\
         measured_LRA={:.2}:measured_thresh={:.2}:offset={:.2}:linear={linear_mode}:\
         print_format=summary",
        opts.target_i,
        opts.target_tp,
        opts.target_lra,
        m.input_i,
        m.input_tp,
        m.input_lra,
        m.input_thresh,
        m.target_offset,
    );

    if opts.precompress {
        info!("Precompress enabled: prepending acompressor (threshold=-18dB, ratio=3:1)");
        format!(
            "acompressor=threshold=0.125893:ratio=3:attack=20:release=200:makeup=2:knee=6,\
             {loudnorm}"
        )
    } else {
        loudnorm
    }
}

impl FFmpegRunner {
    /// Normalize audio levels in a media file.
    ///
    /// Video streams are copied without re-encoding. Audio is decoded,
    /// filtered (volume/limiter or loudnorm), and re-encoded with an
    /// appropriate codec for the output container.
    ///
    /// When `opts.salvage` is true (default), corrupt Matroska/WebM containers
    /// are automatically detected and remuxed to a clean temporary file before
    /// normalization. This prevents EBML structural errors from cascading into
    /// muxer ENOMEM failures during the encode pipeline.
    pub async fn normalize_audio(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("normalize_audio", move || {
            // Check for container corruption and optionally salvage
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, opts.salvage)?;

            let result = match opts.mode {
                AudioNormMode::Peak => Self::normalize_peak_sync(&effective_input, &output, &opts),
                AudioNormMode::Loudnorm => {
                    Self::normalize_loudnorm_sync(&effective_input, &output, &opts)
                }
            };

            // Clean up salvage temp file regardless of success/failure
            if let Some(ref temp) = salvage_temp {
                let _ = std::fs::remove_file(temp);
            }

            result
        })
        .await
    }

    /// Peak normalization: analyze then apply gain + limiter.
    fn normalize_peak_sync(input: &Path, output: &Path, opts: &NormalizeOptions) -> Result<()> {
        let analysis = Self::analyze_peak_sync(input, opts.target_peak_db)?;

        info!(
            "Peak analysis: peak={:.1} dBFS, RMS={:.1} dBFS, gain={:.1} dB",
            analysis.peak_db, analysis.rms_db, analysis.gain_db
        );

        // Skip if gain adjustment is negligible
        if analysis.gain_db.abs() < 0.5 {
            info!("Audio already near target peak, skipping normalization");
            std::fs::copy(input, output).map_err(|e| PostProcessError::IoError {
                message: format!("failed to copy file: {e}"),
                source: e,
            })?;
            return Ok(());
        }

        Self::apply_peak_gain_sync(input, output, &analysis, opts)
    }

    /// Analyze peak and RMS levels using `astats` filter with frame metadata.
    fn analyze_peak_sync(input: &Path, target_peak_db: f64) -> Result<PeakAnalysis> {
        ensure_init()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

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
            ffmpeg_the_third::codec::context::Context::from_parameters(ist.parameters())?;
        set_single_thread_codec(unsafe { decoder_ctx.as_mut_ptr() });
        let mut decoder = decoder_ctx.decoder().audio()?;

        // Build astats filter graph
        let mut graph = ffmpeg_the_third::filter::Graph::new();
        let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

        FFmpegRunner::add_abuffer_to_graph(
            &mut graph,
            "in",
            ist_time_base,
            decoder.rate(),
            decoder.format().name(),
            &decoder.ch_layout().description(),
        )?;
        graph
            .add(&abuffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffersink filter: {e}"),
            })?;

        let astats_spec = "astats=metadata=1:reset=0:measure_perchannel=none:measure_overall=Peak_level+RMS_level";
        FFmpegRunner::parse_and_validate_filter_graph(&mut graph, "in", "out", astats_spec)?;

        // Skip non-audio streams to avoid allocating memory for large video packets
        Self::discard_non_audio_streams(&mut ictx, ist_index);

        // Decode and filter all frames
        let mut peak_db = f64::NEG_INFINITY;
        let mut rms_db = f64::NEG_INFINITY;

        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        let mut filtered = ffmpeg_the_third::frame::Audio::empty();
        let mut packets_skipped = 0u64;

        // Suppress FFmpeg's C-level decoder error spam during decode loop —
        // we handle errors at the Rust level with rate-limited warnings.
        let _log_suppress = LogSuppressGuard::new();

        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to read packet: {e}"),
            })?;
            if stream.index() != ist_index {
                continue;
            }
            if let Err(e) = decoder.send_packet(&packet) {
                if packets_skipped == 0 {
                    warn!("Audio decoder error during analysis (skipping affected packets): {e}");
                }
                packets_skipped += 1;
                // Clear internal decoder buffer
                while decoder.receive_frame(&mut frame).is_ok() {}
                continue;
            }
            while decoder.receive_frame(&mut frame).is_ok() {
                graph
                    .get("in")
                    .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                    .source()
                    .add(&frame)?;

                loop {
                    let mut out_node = graph.get("out").ok_or_else(|| {
                        PostProcessError::ffmpeg_failed("filter node 'out' not found")
                    })?;
                    if out_node.sink().frame(&mut filtered).is_err() {
                        break;
                    }
                    // Read metadata from filtered frame
                    if let Some(p) = read_frame_metadata(
                        unsafe { filtered.as_ptr() },
                        "lavfi.astats.Overall.Peak_level",
                    ) {
                        peak_db = p;
                    }
                    if let Some(r) = read_frame_metadata(
                        unsafe { filtered.as_ptr() },
                        "lavfi.astats.Overall.RMS_level",
                    ) {
                        rms_db = r;
                    }
                }
            }
        }

        if packets_skipped > 0 {
            warn!(
                "Skipped {packets_skipped} audio packet(s) during peak analysis due to decoder errors"
            );
        }

        // Flush decoder — send_eof may fail if decoder is in a broken state
        if let Err(e) = decoder.send_eof() {
            warn!("Decoder send_eof failed during analysis (continuing with flush): {e}");
        }
        while decoder.receive_frame(&mut frame).is_ok() {
            graph
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)?;

            loop {
                let mut out_node = graph.get("out").ok_or_else(|| {
                    PostProcessError::ffmpeg_failed("filter node 'out' not found")
                })?;
                if out_node.sink().frame(&mut filtered).is_err() {
                    break;
                }
                if let Some(p) = read_frame_metadata(
                    unsafe { filtered.as_ptr() },
                    "lavfi.astats.Overall.Peak_level",
                ) {
                    peak_db = p;
                }
                if let Some(r) = read_frame_metadata(
                    unsafe { filtered.as_ptr() },
                    "lavfi.astats.Overall.RMS_level",
                ) {
                    rms_db = r;
                }
            }
        }

        // Flush filter
        graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()?;
        loop {
            let mut out_node = graph
                .get("out")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            if let Some(p) = read_frame_metadata(
                unsafe { filtered.as_ptr() },
                "lavfi.astats.Overall.Peak_level",
            ) {
                peak_db = p;
            }
            if let Some(r) = read_frame_metadata(
                unsafe { filtered.as_ptr() },
                "lavfi.astats.Overall.RMS_level",
            ) {
                rms_db = r;
            }
        }

        if peak_db == f64::NEG_INFINITY {
            return Err(PostProcessError::NormalizationFailed {
                message: "could not determine peak level from astats metadata".into(),
            });
        }

        let gain_db = target_peak_db - peak_db;

        Ok(PeakAnalysis {
            peak_db,
            rms_db,
            gain_db,
        })
    }

    /// Apply peak gain normalization: encode audio to temp, merge with video.
    ///
    /// Same two-step approach as loudnorm pass 2: audio-only encode then merge.
    /// When `opts.salvage` is true, wraps the encode with `with_mux_retry` for
    /// two-tier recovery (salvage remux → CLI fallback) on mux write failures.
    fn apply_peak_gain_sync(
        input: &Path,
        output: &Path,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        ensure_init()?;

        let has_video = {
            let ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open input {}: {e}", input.display()),
                }
            })?;
            ictx.streams()
                .best(ffmpeg_the_third::media::Type::Video)
                .is_some()
        };

        let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mp4");

        if has_video {
            let audio_ext = audio_only_extension_for(ext);
            let temp_audio = output.with_extension(format!("norm_audio.{audio_ext}"));

            if opts.salvage {
                with_mux_retry(
                    input,
                    &temp_audio,
                    |effective_input| {
                        Self::peak_encode_audio_only(
                            effective_input,
                            &temp_audio,
                            ext,
                            analysis,
                            opts,
                        )
                    },
                    |fallback_in, fallback_out| {
                        cli_fallback_peak(fallback_in, fallback_out, analysis, opts, ext)
                    },
                )?;
            } else {
                Self::peak_encode_audio_only(input, &temp_audio, ext, analysis, opts)?;
            }
            let merge_result =
                Self::merge_sync(input, &temp_audio, output, &super::RemuxOptions::default());
            let _ = std::fs::remove_file(&temp_audio);
            merge_result
        } else if opts.salvage {
            with_mux_retry(
                input,
                output,
                |effective_input| {
                    Self::peak_encode_audio_only(effective_input, output, ext, analysis, opts)
                },
                |fallback_in, fallback_out| {
                    cli_fallback_peak(fallback_in, fallback_out, analysis, opts, ext)
                },
            )
        } else {
            Self::peak_encode_audio_only(input, output, ext, analysis, opts)
        }
    }

    /// Encode peak-normalized audio to an output file (video streams discarded).
    #[allow(clippy::too_many_lines)]
    fn peak_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        ensure_init()?;

        let gain_db = opts.target_peak_db - analysis.peak_db;
        let linear_limit = 10f64.powf(opts.target_peak_db / 20.0);

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let audio_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ist_time_base = ictx
            .stream(audio_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "audio input stream {audio_ist_index} not found"
                ))
            })?
            .time_base();

        let audio_ist = ictx.stream(audio_ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!(
                "audio input stream {audio_ist_index} not found"
            ))
        })?;
        let mut audio_dec_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(audio_ist.parameters())?;
        set_single_thread_codec(unsafe { audio_dec_ctx.as_mut_ptr() });
        let mut audio_decoder = audio_dec_ctx.decoder().audio()?;

        let input_audio_bitrate = audio_ist.parameters().bit_rate() as usize;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        // Use final_output_ext for encoder selection (not temp file ext) to ensure
        // correct codec for stream copy during merge (e.g., AAC for MP4, Opus for MKV).
        let enc_name = select_audio_encoder_for_container(final_output_ext);
        let enc_codec = ffmpeg_the_third::encoder::find_by_name(enc_name).ok_or_else(|| {
            PostProcessError::UnsupportedCodec {
                codec: enc_name.to_string(),
                operation: "audio normalization".into(),
            }
        })?;

        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        // Audio-only output — no video stream
        let audio_ost_index;
        let audio_enc_context;
        {
            let ost =
                octx.add_stream(enc_codec)
                    .map_err(|e| PostProcessError::FFmpegLibraryError {
                        message: format!("failed to add audio output stream: {e}"),
                    })?;
            audio_ost_index = ost.index();
            audio_enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }

        let mut audio_encoder = audio_enc_context.encoder().audio()?;
        let target_format = Self::pick_audio_sample_format(&enc_codec, audio_decoder.format());
        audio_encoder.set_format(target_format);
        audio_encoder.set_rate(audio_decoder.rate() as i32);
        let enc_time_base = ffmpeg_the_third::Rational(1, audio_decoder.rate() as i32);
        audio_encoder.set_time_base(enc_time_base);

        let channels = audio_decoder.ch_layout().channels();
        Self::set_default_channel_layout(unsafe { audio_encoder.as_mut_ptr() }, channels as i32);

        let target_bitrate = if input_audio_bitrate > 0 {
            input_audio_bitrate
        } else {
            default_bitrate_for_encoder(enc_name)
        };
        audio_encoder.set_bit_rate(target_bitrate);

        if needs_global_header {
            Self::set_global_header_flag(unsafe { audio_encoder.as_mut_ptr() });
        }
        set_single_thread_codec(unsafe { audio_encoder.as_mut_ptr() });

        let mut audio_encoder =
            audio_encoder
                .open_as(enc_codec)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open audio encoder: {e}"),
                })?;

        // Read actual encoder time_base via FFI (may differ from configured after open)
        // SAFETY: audio_encoder is a valid opened encoder context.
        let enc_time_base = unsafe {
            let tb = (*audio_encoder.as_ptr()).time_base;
            ffmpeg_the_third::Rational(tb.num, tb.den)
        };
        debug!(
            "Peak encoder time_base: configured=1/{}, actual={}/{}",
            audio_decoder.rate(),
            enc_time_base.numerator(),
            enc_time_base.denominator(),
        );

        Self::copy_encoder_params_to_stream(&mut octx, audio_ost_index, unsafe {
            audio_encoder.as_ptr()
        });

        // Set avoid_negative_ts for timestamp normalization (matches merge.rs/remux.rs)
        unsafe {
            (*octx.as_mut_ptr()).avoid_negative_ts =
                ffmpeg_the_third::ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;
        }

        // A2: Flush packets to AVIO immediately — prevents Matroska cluster buffering stalls.
        unsafe {
            (*octx.as_mut_ptr()).flags |= ffmpeg_the_third::ffi::AVFMT_FLAG_FLUSH_PACKETS;
        }

        // For audio-only output, set max_interleave_delta = 0 to disable the
        // muxer's interleave queue entirely. Prevents residual buffer growth
        // that can cause ENOMEM on certain MKV files.
        unsafe {
            (*octx.as_mut_ptr()).max_interleave_delta = 0;
        }

        let mut muxer_opts = ffmpeg_the_third::Dictionary::new();
        muxer_opts.set("cluster_time_limit", "500");
        octx.write_header_with(muxer_opts)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // E1: Log threading knobs and mux flags
        {
            let (dec_tc, dec_att) =
                codec_threading_info(unsafe { audio_decoder.as_ptr() });
            let (enc_tc, enc_att) =
                codec_threading_info(unsafe { audio_encoder.as_ptr() });
            let mux_flags = unsafe { (*octx.as_mut_ptr()).flags };
            let pb_buf_size = unsafe {
                let pb = (*octx.as_mut_ptr()).pb;
                if !pb.is_null() { (*pb).buffer_size } else { 0 }
            };
            info!(
                "[peak encode] decoder: thread_count={dec_tc}, active_thread_type={dec_att}; \
                 encoder: thread_count={enc_tc}, active_thread_type={enc_att}; \
                 mux_flags=0x{mux_flags:x}; pb_buffer_size={pb_buf_size}",
            );
        }

        // Log AVIO state for diagnostics after header write
        // SAFETY: octx owns a valid, header-written output format context.
        unsafe { dump_io_state(octx.as_mut_ptr(), output, "peak_encode_audio_only") };

        // Fail-fast: reject CUSTOM_IO (expected file-based IO)
        {
            let flags = unsafe { (*octx.as_mut_ptr()).flags };
            if flags & ffmpeg_the_third::ffi::AVFMT_FLAG_CUSTOM_IO != 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "[peak_encode_audio_only] AVFMT_FLAG_CUSTOM_IO is set — expected file-based IO".into(),
                });
            }
        }

        // Fail-fast: assert seekable IO context for Matroska (needs seeking for Cues/SeekHead)
        unsafe {
            assert_seekable_io(octx.as_mut_ptr(), "peak_encode_audio_only")?;
        }

        // Flush AVIO after header write to ensure bytes reach disk
        unsafe {
            let pb = (*octx.as_mut_ptr()).pb;
            if !pb.is_null() {
                ffmpeg_the_third::ffi::avio_flush(pb);
            }
        }
        let file_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        if file_size == 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: "[peak_encode_audio_only] output file is 0 bytes after header write + avio_flush — IO sink is broken".into(),
            });
        }
        info!("[peak_encode_audio_only] header written: {file_size} bytes on disk");

        // Build audio filter: volume → alimiter → aresample → aformat → abuffersink
        // The alimiter filter only supports dblp format. An explicit aresample
        // bridges dblp→encoder format since FFmpeg 8.0's auto-conversion during
        // graph_config may fail (same issue as loudnorm, see pass 1 comment).
        let enc_ch_layout_desc = audio_encoder.ch_layout().description();
        let filter_spec = format!(
            "volume={gain_db:.6}dB,alimiter=limit={linear_limit:.6}:attack=5:release=50,aresample,aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
            audio_encoder.format().name(),
            audio_encoder.rate(),
            enc_ch_layout_desc,
        );

        let mut filter_graph =
            build_audio_filter_with_spec(&audio_decoder, audio_ist_time_base, &filter_spec)?;

        // Tell buffersink to output exactly frame_size samples per frame.
        // The last frame at EOF is zero-padded. Required for fixed-frame-size
        // encoders (AAC=1024, MP3=1152, Opus=960).
        Self::set_buffersink_frame_size(&mut filter_graph, "out", audio_encoder.frame_size());

        // Discard non-audio streams to avoid ENOMEM on large video packets
        Self::discard_non_audio_streams(&mut ictx, audio_ist_index);

        // Read ost_time_base AFTER write_header (Matroska may change it)
        let ost_time_base = octx
            .stream(audio_ost_index)
            .ok_or_else(|| PostProcessError::ffmpeg_failed("output stream not found"))?
            .time_base();

        let expected_duration = if audio_encoder.frame_size() > 0 {
            unsafe {
                ffmpeg_the_third::ffi::av_rescale_q(
                    i64::from(audio_encoder.frame_size()),
                    ffmpeg_the_third::ffi::AVRational {
                        num: enc_time_base.numerator(),
                        den: enc_time_base.denominator(),
                    },
                    ffmpeg_the_third::ffi::AVRational {
                        num: ost_time_base.numerator(),
                        den: ost_time_base.denominator(),
                    },
                )
            }
        } else {
            0
        };
        info!(
            "Expected audio packet duration: {} (frame_size={}, enc_tb={}/{}, ost_tb={}/{})",
            expected_duration, audio_encoder.frame_size(),
            enc_time_base.numerator(), enc_time_base.denominator(),
            ost_time_base.numerator(), ost_time_base.denominator(),
        );

        // Suppress decoder WARNING spam while keeping muxer ERRORs visible.
        // Uses error_level() instead of new() so Matroska muxer errors are diagnosable.
        let _log_suppress = LogSuppressGuard::error_level();

        let mut timing = MuxTimingState {
            encoder_frame_size: audio_encoder.frame_size(),
            expected_duration,
            sample_rate: audio_decoder.rate(),
            use_sample_clock: true,
            ..Default::default()
        };

        // Transcode loop (audio only)
        let mut packets_processed = 0u64;
        let mut packets_skipped = 0u64;
        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to read packet: {e}"),
            })?;
            if stream.index() != audio_ist_index {
                continue;
            }
            if let Err(e) = audio_decoder.send_packet(&packet) {
                if packets_skipped == 0 {
                    // ENOMEM = 12 on all POSIX platforms and Windows CRT
                    if matches!(&e, ffmpeg_the_third::Error::Other { errno } if *errno == 12) {
                        warn!(
                            "Audio decoder allocation failure (ENOMEM) — \
                             process may be running out of memory"
                        );
                    } else {
                        warn!("Audio decoder error (skipping affected packets): {e}");
                    }
                }
                packets_skipped += 1;
                // Drain decoder to clear its internal buffer — send_packet buffers
                // the packet before attempting decode, so receive_frame must be
                // called to consume it even on failure.
                if let Err(drain_err) = Self::receive_and_process_audio_direct(
                    &mut audio_decoder,
                    &mut filter_graph,
                    &mut audio_encoder,
                    &mut octx,
                    audio_ost_index,
                    enc_time_base,
                    &mut timing,
                    Some(output),
                ) {
                    // Propagate MuxWriteError directly for salvage retry eligibility
                    if drain_err.is_mux_write_error() {
                        return Err(drain_err);
                    }
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!(
                            "(peak encode) mux/encode pipeline failed while draining after decoder error: {drain_err}"
                        ),
                    });
                }
                continue;
            }
            packets_processed += 1;
            Self::receive_and_process_audio_direct(
                &mut audio_decoder,
                &mut filter_graph,
                &mut audio_encoder,
                &mut octx,
                audio_ost_index,
                enc_time_base,
                &mut timing,
                Some(output),
            )?;
        }

        if packets_skipped > 0 {
            warn!(
                "Skipped {packets_skipped} of {} audio packet(s) due to decoder errors",
                packets_processed + packets_skipped,
            );
        }

        // If every packet failed, the decoder is in a broken state and the
        // output would be empty/corrupt. Bail out so the caller can fall back
        // to copying the file unchanged.
        if packets_processed == 0 && packets_skipped > 0 {
            return Err(PostProcessError::NormalizationFailed {
                message: format!(
                    "audio decoder failed on all {packets_skipped} packets — cannot normalize"
                ),
            });
        }

        // Flush — send_eof may fail if decoder encountered persistent errors
        if let Err(e) = audio_decoder.send_eof() {
            warn!("Decoder send_eof failed (continuing with flush): {e}");
        }
        Self::receive_and_process_audio_direct(
            &mut audio_decoder,
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_filter_to_encoder_direct(
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        audio_encoder.send_eof()?;
        Self::drain_encoder_packets_direct(
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        // Drop suppress guard before write_trailer so any trailer errors are visible
        drop(_log_suppress);

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// EBU R128 loudnorm two-pass normalization.
    fn normalize_loudnorm_sync(input: &Path, output: &Path, opts: &NormalizeOptions) -> Result<()> {
        info!("Loudnorm pass 1: analyzing EBU R128 levels...");
        let measurements = Self::loudnorm_pass1_sync(input, opts)?;

        info!(
            "Loudnorm measurements: I={:.1} LUFS, TP={:.1} dBTP, LRA={:.1} LU",
            measurements.input_i, measurements.input_tp, measurements.input_lra
        );

        if measurements.input_i < -35.0 {
            warn!(
                "Very quiet source ({:.1} LUFS) — normalization will amplify noise",
                measurements.input_i,
            );
        }

        info!("Loudnorm pass 2: applying normalization...");
        Self::loudnorm_pass2_sync(input, output, opts, &measurements)?;

        // Verify output loudness against targets
        Self::verify_loudness_sync(output, opts)?;

        Ok(())
    }

    /// Post-normalization loudness verification.
    ///
    /// Runs loudnorm pass 1 on the **output** file and compares measured
    /// levels against targets. Warns on significant deviations but does
    /// not fail — the output is already written.
    fn verify_loudness_sync(output: &Path, opts: &NormalizeOptions) -> Result<()> {
        info!("Loudness verification: analyzing output...");
        match Self::loudnorm_pass1_sync(output, opts) {
            Ok(measured) => {
                info!(
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

    /// Loudnorm pass 1: run loudnorm filter in analysis mode, capture JSON from logs.
    fn loudnorm_pass1_sync(input: &Path, opts: &NormalizeOptions) -> Result<LoudnormMeasurements> {
        ensure_init()?;

        let guard = LogCaptureGuard::begin()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let ist = ictx.stream(ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
        })?;
        let ist_time_base = ist.time_base();

        let mut decoder_ctx = ffmpeg_the_third::codec::context::Context::from_parameters(
            ist.parameters(),
        )
        .map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to create decoder context: {e}"),
        })?;
        // B1: Single-threaded decode for pass 1 — reduces RSS during analysis.
        set_single_thread_codec(unsafe { decoder_ctx.as_mut_ptr() });
        let mut decoder =
            decoder_ctx
                .decoder()
                .audio()
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open audio decoder: {e}"),
                })?;

        debug!(
            "loudnorm pass 1 decoder: rate={}, fmt={}, ch_layout={}, time_base={}/{}",
            decoder.rate(),
            decoder.format().name(),
            decoder.ch_layout().description(),
            ist_time_base.numerator(),
            ist_time_base.denominator(),
        );

        // Build loudnorm analysis filter
        // loudnorm only supports AV_SAMPLE_FMT_DBL; explicitly convert from
        // decoder format (typically fltp) since FFmpeg 8.0's auto-conversion
        // during graph_config may fail with EINVAL.
        let loudnorm_spec = format!(
            "aformat=sample_fmts=dbl,loudnorm=I={:.1}:TP={:.1}:LRA={:.1}:print_format=json",
            opts.target_i, opts.target_tp, opts.target_lra,
        );

        let mut graph = ffmpeg_the_third::filter::Graph::new();
        let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

        FFmpegRunner::add_abuffer_to_graph(
            &mut graph,
            "in",
            ist_time_base,
            decoder.rate(),
            decoder.format().name(),
            &decoder.ch_layout().description(),
        )?;
        graph
            .add(&abuffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffersink filter: {e}"),
            })?;

        FFmpegRunner::parse_and_validate_filter_graph(&mut graph, "in", "out", &loudnorm_spec)?;

        // Skip non-audio streams to avoid allocating memory for large video packets
        Self::discard_non_audio_streams(&mut ictx, ist_index);

        // Process all audio frames (discard output, we just need the log JSON)
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        let mut filtered = ffmpeg_the_third::frame::Audio::empty();
        let mut packets_skipped = 0u64;

        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to read packet: {e}"),
            })?;
            if stream.index() != ist_index {
                continue;
            }
            if let Err(e) = decoder.send_packet(&packet) {
                if packets_skipped == 0 {
                    warn!(
                        "Audio decoder error during loudnorm analysis (skipping affected packets): {e}"
                    );
                }
                packets_skipped += 1;
                // Clear internal decoder buffer
                while decoder.receive_frame(&mut frame).is_ok() {}
                continue;
            }
            while decoder.receive_frame(&mut frame).is_ok() {
                graph
                    .get("in")
                    .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                    .source()
                    .add(&frame)
                    .map_err(|e| PostProcessError::FFmpegLibraryError {
                        message: format!("filter source add frame failed: {e}"),
                    })?;
                // B3: Release decode buffer ref immediately — filter holds its own.
                frame_unref_audio(&mut frame);

                // Drain filter output (discard frames)
                loop {
                    let mut out_node = graph.get("out").ok_or_else(|| {
                        PostProcessError::ffmpeg_failed("filter node 'out' not found")
                    })?;
                    if out_node.sink().frame(&mut filtered).is_err() {
                        break;
                    }
                    frame_unref_audio(&mut filtered);
                }
            }
        }

        if packets_skipped > 0 {
            warn!(
                "Skipped {packets_skipped} audio packet(s) during loudnorm analysis due to decoder errors"
            );
        }

        // Flush decoder — send_eof may fail if decoder is in a broken state
        if let Err(e) = decoder.send_eof() {
            warn!("Decoder send_eof failed during loudnorm analysis (continuing with flush): {e}");
        }
        while decoder.receive_frame(&mut frame).is_ok() {
            graph
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("filter source add frame (flush) failed: {e}"),
                })?;
            frame_unref_audio(&mut frame);

            loop {
                let mut out_node = graph.get("out").ok_or_else(|| {
                    PostProcessError::ffmpeg_failed("filter node 'out' not found")
                })?;
                if out_node.sink().frame(&mut filtered).is_err() {
                    break;
                }
                frame_unref_audio(&mut filtered);
            }
        }

        // Flush filter
        graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("filter source flush failed: {e}"),
            })?;
        loop {
            let mut out_node = graph
                .get("out")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            frame_unref_audio(&mut filtered);
        }

        // Drop the filter graph to trigger loudnorm's uninit(), which emits
        // the JSON measurements via av_log at AV_LOG_INFO level.
        drop(graph);

        // Now capture the log output (JSON was emitted during graph drop)
        let lines = guard.take_captured()?;
        drop(guard);

        debug!("Captured {} log lines from loudnorm pass 1", lines.len());

        parse_loudnorm_json(&lines)
    }

    /// Loudnorm pass 2: apply normalization with measured values.
    ///
    /// For files with video, uses a two-step approach to avoid ENOMEM from
    /// large video packets: (1) encode normalized audio to a temp file with
    /// video discarded, (2) merge original video + normalized audio via
    /// stream copy.
    ///
    /// When `opts.salvage` is true, wraps the encode with `with_mux_retry` for
    /// two-tier recovery (salvage remux → CLI fallback) on mux write failures.
    fn loudnorm_pass2_sync(
        input: &Path,
        output: &Path,
        opts: &NormalizeOptions,
        measurements: &LoudnormMeasurements,
    ) -> Result<()> {
        ensure_init()?;

        // Check if input has video
        let has_video = {
            let ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open input {}: {e}", input.display()),
                }
            })?;
            ictx.streams()
                .best(ffmpeg_the_third::media::Type::Video)
                .is_some()
        };

        let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mp4");

        if has_video {
            // Two-step: audio-only encode → merge with original video
            let audio_ext = audio_only_extension_for(ext);
            let temp_audio = output.with_extension(format!("norm_audio.{audio_ext}"));

            if opts.salvage {
                with_mux_retry(
                    input,
                    &temp_audio,
                    |effective_input| {
                        Self::loudnorm_encode_audio_only(
                            effective_input,
                            &temp_audio,
                            ext,
                            opts,
                            measurements,
                        )
                    },
                    |fallback_in, fallback_out| {
                        cli_fallback_loudnorm(fallback_in, fallback_out, opts, measurements, ext)
                    },
                )?;
            } else {
                Self::loudnorm_encode_audio_only(input, &temp_audio, ext, opts, measurements)?;
            }
            let merge_result =
                Self::merge_sync(input, &temp_audio, output, &super::RemuxOptions::default());
            let _ = std::fs::remove_file(&temp_audio);
            merge_result
        } else if opts.salvage {
            with_mux_retry(
                input,
                output,
                |effective_input| {
                    Self::loudnorm_encode_audio_only(
                        effective_input,
                        output,
                        ext,
                        opts,
                        measurements,
                    )
                },
                |fallback_in, fallback_out| {
                    cli_fallback_loudnorm(fallback_in, fallback_out, opts, measurements, ext)
                },
            )
        } else {
            // Audio-only file: encode directly to output
            Self::loudnorm_encode_audio_only(input, output, ext, opts, measurements)
        }
    }

    /// Encode normalized audio to an output file (video streams discarded).
    #[allow(clippy::too_many_lines)]
    fn loudnorm_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        opts: &NormalizeOptions,
        measurements: &LoudnormMeasurements,
    ) -> Result<()> {
        ensure_init()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let audio_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ist_time_base = ictx
            .stream(audio_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "audio input stream {audio_ist_index} not found"
                ))
            })?
            .time_base();

        let audio_ist = ictx.stream(audio_ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!(
                "audio input stream {audio_ist_index} not found"
            ))
        })?;
        let mut audio_dec_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(audio_ist.parameters())?;
        // B1: Force single-threaded decode — eliminates frame-threading buffer
        // pre-allocation that inflates RSS by hundreds of MB on long audio.
        set_single_thread_codec(unsafe { audio_dec_ctx.as_mut_ptr() });
        let mut audio_decoder = audio_dec_ctx.decoder().audio()?;

        let input_audio_bitrate = audio_ist.parameters().bit_rate() as usize;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        // Use final_output_ext for encoder selection (not temp file ext) to ensure
        // correct codec for stream copy during merge (e.g., AAC for MP4, Opus for MKV).
        let enc_name = select_audio_encoder_for_container(final_output_ext);
        let enc_codec = ffmpeg_the_third::encoder::find_by_name(enc_name).ok_or_else(|| {
            PostProcessError::UnsupportedCodec {
                codec: enc_name.to_string(),
                operation: "audio normalization (loudnorm)".into(),
            }
        })?;

        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        // Audio-only output — no video stream
        let audio_ost_index;
        let audio_enc_context;
        {
            let ost =
                octx.add_stream(enc_codec)
                    .map_err(|e| PostProcessError::FFmpegLibraryError {
                        message: format!("failed to add audio output stream: {e}"),
                    })?;
            audio_ost_index = ost.index();
            audio_enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }

        let mut audio_encoder = audio_enc_context.encoder().audio()?;
        let target_format = Self::pick_audio_sample_format(&enc_codec, audio_decoder.format());
        audio_encoder.set_format(target_format);
        audio_encoder.set_rate(audio_decoder.rate() as i32);
        let enc_time_base = ffmpeg_the_third::Rational(1, audio_decoder.rate() as i32);
        audio_encoder.set_time_base(enc_time_base);

        let channels = audio_decoder.ch_layout().channels();
        Self::set_default_channel_layout(unsafe { audio_encoder.as_mut_ptr() }, channels as i32);

        let target_bitrate = if input_audio_bitrate > 0 {
            input_audio_bitrate
        } else {
            default_bitrate_for_encoder(enc_name)
        };
        audio_encoder.set_bit_rate(target_bitrate);

        if needs_global_header {
            Self::set_global_header_flag(unsafe { audio_encoder.as_mut_ptr() });
        }

        // B2: Force single-threaded encode — reduces encoder buffer pool memory.
        set_single_thread_codec(unsafe { audio_encoder.as_mut_ptr() });

        let mut audio_encoder =
            audio_encoder
                .open_as(enc_codec)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open audio encoder: {e}"),
                })?;

        // Read actual encoder time_base via FFI (may differ from configured after open)
        // SAFETY: audio_encoder is a valid opened encoder context.
        let enc_time_base = unsafe {
            let tb = (*audio_encoder.as_ptr()).time_base;
            ffmpeg_the_third::Rational(tb.num, tb.den)
        };
        debug!(
            "Loudnorm encoder time_base: configured=1/{}, actual={}/{}",
            audio_decoder.rate(),
            enc_time_base.numerator(),
            enc_time_base.denominator(),
        );

        Self::copy_encoder_params_to_stream(&mut octx, audio_ost_index, unsafe {
            audio_encoder.as_ptr()
        });

        // Set avoid_negative_ts for timestamp normalization (matches merge.rs/remux.rs)
        unsafe {
            (*octx.as_mut_ptr()).avoid_negative_ts =
                ffmpeg_the_third::ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;
        }

        // A2: Flush packets to AVIO immediately — prevents Matroska cluster buffering stalls.
        unsafe {
            (*octx.as_mut_ptr()).flags |= ffmpeg_the_third::ffi::AVFMT_FLAG_FLUSH_PACKETS;
        }

        // A1: For audio-only output using av_write_frame (non-interleaved), set
        // max_interleave_delta = 0 to disable the muxer's interleave queue entirely.
        // Harmless when using direct writes; prevents any residual queue growth.
        unsafe {
            (*octx.as_mut_ptr()).max_interleave_delta = 0;
        }

        let mut muxer_opts = ffmpeg_the_third::Dictionary::new();
        muxer_opts.set("cluster_time_limit", "500");
        octx.write_header_with(muxer_opts)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // E1: Log threading knobs, mux flags, and AVIO state for diagnostics
        {
            let (dec_tc, dec_att) =
                codec_threading_info(unsafe { audio_decoder.as_ptr() });
            let (enc_tc, enc_att) =
                codec_threading_info(unsafe { audio_encoder.as_ptr() });
            let mux_flags = unsafe { (*octx.as_mut_ptr()).flags };
            let pb_buf_size = unsafe {
                let pb = (*octx.as_mut_ptr()).pb;
                if !pb.is_null() { (*pb).buffer_size } else { 0 }
            };
            info!(
                "[loudnorm pass 2] decoder: thread_count={dec_tc}, active_thread_type={dec_att}; \
                 encoder: thread_count={enc_tc}, active_thread_type={enc_att}; \
                 mux_flags=0x{mux_flags:x} (flush_packets={}); \
                 pb_buffer_size={pb_buf_size}",
                (mux_flags & ffmpeg_the_third::ffi::AVFMT_FLAG_FLUSH_PACKETS) != 0,
            );
        }

        // Log AVIO state for diagnostics after header write
        // SAFETY: octx owns a valid, header-written output format context.
        unsafe { dump_io_state(octx.as_mut_ptr(), output, "loudnorm_encode_audio_only") };

        // Fail-fast: reject CUSTOM_IO (expected file-based IO)
        {
            let flags = unsafe { (*octx.as_mut_ptr()).flags };
            if flags & ffmpeg_the_third::ffi::AVFMT_FLAG_CUSTOM_IO != 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "[loudnorm_encode_audio_only] AVFMT_FLAG_CUSTOM_IO is set — expected file-based IO".into(),
                });
            }
        }

        // Fail-fast: assert seekable IO context for Matroska (needs seeking for Cues/SeekHead)
        unsafe {
            assert_seekable_io(octx.as_mut_ptr(), "loudnorm_encode_audio_only")?;
        }

        // Flush AVIO after header write to ensure bytes reach disk
        unsafe {
            let pb = (*octx.as_mut_ptr()).pb;
            if !pb.is_null() {
                ffmpeg_the_third::ffi::avio_flush(pb);
            }
        }
        let file_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        if file_size == 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: "[loudnorm_encode_audio_only] output file is 0 bytes after header write + avio_flush — IO sink is broken".into(),
            });
        }
        info!("[loudnorm_encode_audio_only] header written: {file_size} bytes on disk");

        // Build loudnorm pass 2 filter chain.
        //
        // Chain order matters for true-peak compliance:
        //   aformat=dbl → [precomp] → loudnorm → aresample → alimiter → aformat
        //
        // The alimiter MUST come AFTER aresample because resampling (e.g.
        // 44100→48000 Hz sinc interpolation) can introduce ~1-2 dB of peak
        // overshoot.  Placing the limiter last in the chain guarantees the
        // encoder receives peak-compliant samples.
        //
        // An explicit aresample bridges dbl→encoder format since FFmpeg 8.0's
        // auto-conversion during graph_config may fail with EINVAL (same issue
        // that required the leading aformat=dbl in pass 1).
        // Note: aresample is kept plain (no async/first_pts options) — adding
        // async=0:first_pts=0 caused silent padding that lowered output volume.
        let loudnorm_core = build_loudnorm_pass2_filter(opts, measurements);
        let limiter = build_alimiter_spec(opts.target_tp);
        let enc_ch_layout_desc = audio_encoder.ch_layout().description();
        let filter_spec = format!(
            "aformat=sample_fmts=dbl,{loudnorm_core},aresample,{limiter},\
             aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
            audio_encoder.format().name(),
            audio_encoder.rate(),
            enc_ch_layout_desc,
        );
        info!("Loudnorm pass 2 filter_spec={filter_spec}");

        // Log decoder/encoder audio parameters for diagnostics
        info!(
            "Loudnorm pass 2 decoder: sample_rate={}, format={}, ch_layout={}",
            audio_decoder.rate(),
            audio_decoder.format().name(),
            audio_decoder.ch_layout().description(),
        );
        info!(
            "Loudnorm pass 2 encoder: sample_rate={}, format={}, ch_layout={}",
            audio_encoder.rate(),
            audio_encoder.format().name(),
            enc_ch_layout_desc,
        );

        let mut filter_graph =
            build_audio_filter_with_spec(&audio_decoder, audio_ist_time_base, &filter_spec)?;

        // Tell buffersink to output exactly frame_size samples per frame.
        // The last frame at EOF is zero-padded. Required for fixed-frame-size
        // encoders (AAC=1024, MP3=1152, Opus=960).
        Self::set_buffersink_frame_size(&mut filter_graph, "out", audio_encoder.frame_size());

        // Discard non-audio streams to avoid ENOMEM on large video packets
        Self::discard_non_audio_streams(&mut ictx, audio_ist_index);

        // Read ost_time_base AFTER write_header (Matroska may change it)
        let ost_time_base = octx
            .stream(audio_ost_index)
            .ok_or_else(|| PostProcessError::ffmpeg_failed("output stream not found"))?
            .time_base();

        let expected_duration = if audio_encoder.frame_size() > 0 {
            unsafe {
                ffmpeg_the_third::ffi::av_rescale_q(
                    i64::from(audio_encoder.frame_size()),
                    ffmpeg_the_third::ffi::AVRational {
                        num: enc_time_base.numerator(),
                        den: enc_time_base.denominator(),
                    },
                    ffmpeg_the_third::ffi::AVRational {
                        num: ost_time_base.numerator(),
                        den: ost_time_base.denominator(),
                    },
                )
            }
        } else {
            0
        };
        info!(
            "Expected audio packet duration: {} (frame_size={}, enc_tb={}/{}, ost_tb={}/{})",
            expected_duration, audio_encoder.frame_size(),
            enc_time_base.numerator(), enc_time_base.denominator(),
            ost_time_base.numerator(), ost_time_base.denominator(),
        );

        // Suppress decoder WARNING spam while keeping muxer ERRORs visible.
        // Uses error_level() instead of new() so Matroska muxer errors are diagnosable.
        let _log_suppress = LogSuppressGuard::error_level();

        let mut timing = MuxTimingState {
            encoder_frame_size: audio_encoder.frame_size(),
            expected_duration,
            sample_rate: audio_decoder.rate(),
            use_sample_clock: true,
            ..Default::default()
        };

        // Transcode loop (audio only)
        let mut packets_processed = 0u64;
        let mut packets_skipped = 0u64;
        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to read packet: {e}"),
            })?;
            if stream.index() != audio_ist_index {
                continue;
            }
            if let Err(e) = audio_decoder.send_packet(&packet) {
                if packets_skipped == 0 {
                    // ENOMEM = 12 on all POSIX platforms and Windows CRT
                    if matches!(&e, ffmpeg_the_third::Error::Other { errno } if *errno == 12) {
                        warn!(
                            "Audio decoder allocation failure (ENOMEM) — \
                             process may be running out of memory"
                        );
                    } else {
                        warn!("Audio decoder error (skipping affected packets): {e}");
                    }
                }
                packets_skipped += 1;
                // Drain decoder to clear its internal buffer — send_packet buffers
                // the packet before attempting decode, so receive_frame must be
                // called to consume it even on failure.
                if let Err(drain_err) = Self::receive_and_process_audio_direct(
                    &mut audio_decoder,
                    &mut filter_graph,
                    &mut audio_encoder,
                    &mut octx,
                    audio_ost_index,
                    enc_time_base,
                    &mut timing,
                    Some(output),
                ) {
                    // Propagate MuxWriteError directly for salvage retry eligibility
                    if drain_err.is_mux_write_error() {
                        return Err(drain_err);
                    }
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!(
                            "(loudnorm pass 2) mux/encode pipeline failed while draining after decoder error: {drain_err}"
                        ),
                    });
                }
                continue;
            }
            packets_processed += 1;
            Self::receive_and_process_audio_direct(
                &mut audio_decoder,
                &mut filter_graph,
                &mut audio_encoder,
                &mut octx,
                audio_ost_index,
                enc_time_base,
                &mut timing,
                Some(output),
            )?;
        }

        if packets_skipped > 0 {
            warn!(
                "Skipped {packets_skipped} of {} audio packet(s) due to decoder errors",
                packets_processed + packets_skipped,
            );
        }

        // If every packet failed, the decoder is in a broken state and the
        // output would be empty/corrupt. Bail out so the caller can fall back
        // to copying the file unchanged.
        if packets_processed == 0 && packets_skipped > 0 {
            return Err(PostProcessError::NormalizationFailed {
                message: format!(
                    "audio decoder failed on all {packets_skipped} packets — cannot normalize"
                ),
            });
        }

        // Flush — send_eof may fail if decoder encountered persistent errors
        if let Err(e) = audio_decoder.send_eof() {
            warn!("Decoder send_eof failed (continuing with flush): {e}");
        }
        Self::receive_and_process_audio_direct(
            &mut audio_decoder,
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_filter_to_encoder_direct(
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        audio_encoder.send_eof()?;
        Self::drain_encoder_packets_direct(
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        // Drop suppress guard before write_trailer so any trailer errors are visible
        drop(_log_suppress);

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Comprehensive IO state dump for diagnostics.
///
/// Logs muxer name, format context URL and flags, AVIO buffer state,
/// and the actual file size on disk. Used after `write_header`, before
/// first packet write, and on watchdog stall to capture the full IO
/// picture for debugging.
///
/// # Safety
///
/// `octx_ptr` must point to a valid, header-written `AVFormatContext`.
pub(crate) unsafe fn dump_io_state(
    octx_ptr: *mut ffmpeg_the_third::ffi::AVFormatContext,
    rust_output_path: &Path,
    label: &str,
) {
    unsafe {
        let oformat = (*octx_ptr).oformat;
        let muxer_name = if !oformat.is_null() && !(*oformat).name.is_null() {
            CStr::from_ptr((*oformat).name)
                .to_string_lossy()
                .into_owned()
        } else {
            "unknown".to_string()
        };

        let url = if !(*octx_ptr).url.is_null() {
            CStr::from_ptr((*octx_ptr).url)
                .to_string_lossy()
                .into_owned()
        } else {
            "NULL".to_string()
        };

        let flags = (*octx_ptr).flags;
        let custom_io = (flags & ffmpeg_the_third::ffi::AVFMT_FLAG_CUSTOM_IO) != 0;

        let pb = (*octx_ptr).pb;
        if pb.is_null() {
            info!(
                "[{label}] IO dump: muxer={muxer_name}, url={url}, flags=0x{flags:x}, \
                 custom_io={custom_io}, pb=NULL"
            );
            return;
        }

        let seekable = (*pb).seekable;
        let buffer_size = (*pb).buffer_size;
        let pos = (*pb).pos;
        let error = (*pb).error;
        let direct = (*pb).direct;
        let write_flag = (*pb).write_flag;
        let has_write_cb = (*pb).write_packet.is_some();

        let file_size = std::fs::metadata(rust_output_path)
            .map(|m| m.len() as i64)
            .unwrap_or(-1);

        info!(
            "[{label}] IO dump: muxer={muxer_name}, url={url}, flags=0x{flags:x}, \
             custom_io={custom_io}, pb={{seekable={seekable}, buffer_size={buffer_size}, \
             pos={pos}, error={error}, direct={direct}, write_flag={write_flag}, \
             write_cb={}}}, rust_path={}, file_size={file_size}",
            if has_write_cb { "present" } else { "NULL" },
            rust_output_path.display(),
        );
    }
}

/// Assert that the output format context has a seekable IO context.
///
/// Matroska requires seeking for Cues/SeekHead; a non-seekable output
/// silently stalls the muxer. This check fails fast after `write_header`
/// so the error is caught before the encode loop.
///
/// # Safety
///
/// `octx_ptr` must point to a valid, header-written `AVFormatContext`.
unsafe fn assert_seekable_io(
    octx_ptr: *mut ffmpeg_the_third::ffi::AVFormatContext,
    label: &str,
) -> crate::error::Result<()> {
    unsafe {
        let pb = (*octx_ptr).pb;
        if pb.is_null() {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!("[{label}] output IO context (pb) is NULL — no output possible"),
            });
        }
        if (*pb).seekable == 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!(
                    "[{label}] output IO context is not seekable (pb->seekable=0) — \
                     Matroska muxer requires seeking for Cues/SeekHead"
                ),
            });
        }
        let oformat = (*octx_ptr).oformat;
        if !oformat.is_null() && ((*oformat).flags & ffmpeg_the_third::ffi::AVFMT_NOFILE) != 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!(
                    "[{label}] output format has AVFMT_NOFILE flag — expected file-based IO"
                ),
            });
        }
        Ok(())
    }
}

/// Read a metadata value from an FFmpeg frame as f64.
///
/// # Safety
///
/// `frame_ptr` must point to a valid `AVFrame`.
fn read_frame_metadata(frame_ptr: *const ffmpeg_the_third::ffi::AVFrame, key: &str) -> Option<f64> {
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
        // Handle "-inf" as negative infinity
        if value.trim() == "-inf" {
            return Some(f64::NEG_INFINITY);
        }
        value.trim().parse::<f64>().ok()
    }
}

/// Select the appropriate audio encoder for a container extension.
fn select_audio_encoder_for_container(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "mp4" | "m4a" | "mov" | "f4v" | "3gp" => "aac",
        "webm" | "ogg" | "opus" => "libopus",
        "mkv" | "mka" => "libopus",
        "ts" | "mpg" => "aac",
        "avi" => "libmp3lame",
        "flv" => "aac",
        "mp3" => "libmp3lame",
        "flac" => "flac",
        "wav" => "pcm_s16le",
        _ => "aac",
    }
}

/// Get a sensible default bitrate (in bps) for an encoder.
fn default_bitrate_for_encoder(encoder: &str) -> usize {
    match encoder {
        "aac" => 128_000,
        "libmp3lame" => 192_000,
        "libopus" => 128_000,
        "flac" | "pcm_s16le" => 0, // Lossless / PCM, no bitrate needed
        _ => 128_000,
    }
}

/// Map a container extension to an audio-only container extension for temp files.
///
/// Uses MKA for all MOV-based formats to avoid the MOV muxer's ENOMEM issue.
/// The MOV muxer accumulates per-packet metadata in memory until trailer write,
/// causing allocation failures on long audio tracks. Matroska writes metadata
/// incrementally without unbounded buffering.
fn audio_only_extension_for(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "mp4" | "m4a" | "mov" | "f4v" | "3gp" | "ts" | "mpg" | "flv" => "mka",
        "mkv" | "mka" | "webm" => "mka",
        "avi" | "mp3" => "mp3",
        "ogg" | "opus" => "opus",
        "flac" => "flac",
        "wav" => "wav",
        _ => "mka",
    }
}

/// Two-tier recovery for mux failures during audio normalization.
///
/// D2: One-shot salvage retry with deterministic cleanup.
/// - Tier 1: Salvage-remux input → retry library encode (one attempt only).
/// - Tier 2: External ffmpeg CLI with `-fflags +discardcorrupt+genpts`.
/// - Never overwrites the original input.
/// - Salvage temp is deleted on success unless `RDLP_KEEP_SALVAGE=1`.
/// - Salvage temp is kept on failure for post-mortem analysis.
///
/// Loudnorm pass 1 measurements remain valid after salvage/CLI because:
/// - Salvage uses stream copy (audio bit-identical)
/// - CLI pass 2 uses the same measured values from pass 1
fn with_mux_retry<F, G>(input: &Path, output: &Path, encode_fn: F, cli_fallback_fn: G) -> Result<()>
where
    F: Fn(&Path) -> Result<()>,
    G: FnOnce(&Path, &Path) -> Result<()>,
{
    let keep_salvage = std::env::var("RDLP_KEEP_SALVAGE")
        .map(|v| v == "1")
        .unwrap_or(false);

    // First attempt: library encode
    match encode_fn(input) {
        Ok(()) => return Ok(()),
        Err(e) if !e.is_salvage_retryable() => {
            // If decoder failed on ALL packets, CLI won't fare better — abort early
            if matches!(&e, PostProcessError::NormalizationFailed { message } if message.contains("all") && message.contains("packets")) {
                warn!("Decoder failed on all packets — skipping salvage/CLI fallback");
            }
            return Err(e);
        }
        Err(e) => {
            warn!("Encode failed with mux error, attempting one-shot salvage retry: {e}");
        }
    }

    // Clean up potentially corrupt partial output before retry
    let _ = std::fs::remove_file(output);
    if output.exists() {
        warn!(
            "Cannot remove partial output {}; skipping salvage, falling to CLI",
            output.display()
        );
        return cli_fallback_fn(input, output);
    }

    // Tier 1: salvage remux → retry library encode (ONE attempt only)
    match salvage_remux_sync(input) {
        Ok(salvaged) => {
            let result = encode_fn(&salvaged);
            if result.is_ok() {
                // D2: Delete salvage temp on success unless RDLP_KEEP_SALVAGE=1
                if keep_salvage {
                    info!(
                        "RDLP_KEEP_SALVAGE=1: keeping salvage temp {}",
                        salvaged.display()
                    );
                } else {
                    let _ = std::fs::remove_file(&salvaged);
                }
                return result;
            }
            warn!(
                "Salvage retry also failed, falling back to CLI: {}",
                result.as_ref().unwrap_err()
            );
            // D2: Keep salvage temp on failure for post-mortem analysis
            if !keep_salvage {
                info!(
                    "Keeping salvage temp for post-mortem: {}",
                    salvaged.display()
                );
            }
            let _ = std::fs::remove_file(output);
            if output.exists() {
                warn!(
                    "Cannot remove failed retry output {}; falling to CLI",
                    output.display()
                );
            }
        }
        Err(e) => {
            warn!("Salvage remux failed, falling back to CLI: {e}");
        }
    }

    // Tier 2: external ffmpeg CLI
    info!("Attempting CLI fallback normalization...");
    cli_fallback_fn(input, output)
}

/// Run peak normalization via external ffmpeg CLI.
///
/// Uses `-fflags +discardcorrupt+genpts` to handle corrupt input that
/// the library cannot process.
fn cli_fallback_peak(
    input: &Path,
    output: &Path,
    analysis: &PeakAnalysis,
    opts: &NormalizeOptions,
    _final_ext: &str,
) -> Result<()> {
    // Use the actual output extension for codec selection — the output is
    // a temp .mka file, not the final container. Using final_ext (e.g. "mkv")
    // could select a codec incompatible with the temp container.
    let output_ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mka");
    let enc_name = select_audio_encoder_for_container(output_ext);
    let linear_limit = 10f64.powf(opts.target_peak_db / 20.0);
    let filter = format!(
        "volume={:.6}dB,alimiter=limit={:.6}:attack=5:release=50",
        analysis.gain_db, linear_limit
    );

    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-y", "-fflags", "+discardcorrupt+genpts"])
        .arg("-i")
        .arg(input)
        .args(["-vn", "-af", &filter])
        .args(["-c:a", enc_name]);
    if enc_name == "libopus" {
        cmd.args(["-ar", "48000"]);
    }
    let cmd_output = cmd
        .arg(output)
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| PostProcessError::FFmpegFailed {
            message: format!("failed to spawn ffmpeg CLI: {e}"),
            source: Some(e),
        })?;

    if !cmd_output.status.success() {
        let stderr = String::from_utf8_lossy(&cmd_output.stderr);
        let excerpt: String = stderr.lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(PostProcessError::NormalizationFailed {
            message: format!(
                "CLI fallback peak normalization failed (exit {}): {}",
                cmd_output.status.code().unwrap_or(-1),
                excerpt,
            ),
        });
    }
    info!("CLI fallback peak normalization succeeded");
    Ok(())
}

/// Run loudnorm two-pass normalization via external ffmpeg CLI.
///
/// Uses `-fflags +discardcorrupt+genpts` to handle corrupt input that
/// the library cannot process. Pass 1 measurements are reused from the
/// prior library analysis.
fn cli_fallback_loudnorm(
    input: &Path,
    output: &Path,
    opts: &NormalizeOptions,
    measurements: &LoudnormMeasurements,
    _final_ext: &str,
) -> Result<()> {
    // Use the actual output extension for codec selection — the output is
    // a temp .mka file, not the final container. Using final_ext (e.g. "mkv")
    // could select a codec incompatible with the temp container.
    let output_ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mka");
    let enc_name = select_audio_encoder_for_container(output_ext);
    // Build filter string — ffmpeg CLI handles format negotiation
    let loudnorm_core = build_loudnorm_pass2_filter(opts, measurements);
    let limiter = build_alimiter_spec(opts.target_tp);
    let filter = format!("{loudnorm_core},{limiter}");
    info!("CLI fallback loudnorm filter: {filter}");

    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-y", "-fflags", "+discardcorrupt+genpts"])
        .arg("-i")
        .arg(input)
        .args(["-vn", "-af", &filter])
        .args(["-c:a", enc_name]);
    if enc_name == "libopus" {
        cmd.args(["-ar", "48000"]);
    }
    let cmd_output = cmd
        .arg(output)
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| PostProcessError::FFmpegFailed {
            message: format!("failed to spawn ffmpeg CLI: {e}"),
            source: Some(e),
        })?;

    if !cmd_output.status.success() {
        let stderr = String::from_utf8_lossy(&cmd_output.stderr);
        let excerpt: String = stderr.lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(PostProcessError::NormalizationFailed {
            message: format!(
                "CLI fallback loudnorm normalization failed (exit {}): {}",
                cmd_output.status.code().unwrap_or(-1),
                excerpt,
            ),
        });
    }
    info!("CLI fallback loudnorm normalization succeeded");
    Ok(())
}

/// Build an audio filter graph with a custom filter spec string.
///
/// Creates: `abuffer → {filter_spec} → abuffersink`
fn build_audio_filter_with_spec(
    decoder: &ffmpeg_the_third::decoder::Audio,
    ist_time_base: ffmpeg_the_third::Rational,
    filter_spec: &str,
) -> Result<ffmpeg_the_third::filter::Graph> {
    let mut graph = ffmpeg_the_third::filter::Graph::new();

    let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
        .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

    FFmpegRunner::add_abuffer_to_graph(
        &mut graph,
        "in",
        ist_time_base,
        decoder.rate(),
        decoder.format().name(),
        &decoder.ch_layout().description(),
    )?;
    graph
        .add(&abuffersink, "out", "")
        .map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to add abuffersink filter: {e}"),
        })?;

    FFmpegRunner::parse_and_validate_filter_graph(&mut graph, "in", "out", filter_spec)?;

    Ok(graph)
}

/// Parse loudnorm JSON output from captured FFmpeg log lines.
///
/// Looks for lines containing `"input_i"`, `"input_tp"`, etc. and extracts
/// the values from the JSON block emitted by `loudnorm print_format=json`.
fn parse_loudnorm_json(lines: &[String]) -> Result<LoudnormMeasurements> {
    // Join all lines and find the JSON block
    let full_text = lines.join("");

    // Extract values by looking for JSON keys
    let input_i = extract_json_value(&full_text, "input_i").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_i' in loudnorm output".into(),
        }
    })?;
    let input_tp = extract_json_value(&full_text, "input_tp").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_tp' in loudnorm output".into(),
        }
    })?;
    let input_lra = extract_json_value(&full_text, "input_lra").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_lra' in loudnorm output".into(),
        }
    })?;
    let input_thresh = extract_json_value(&full_text, "input_thresh").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_thresh' in loudnorm output".into(),
        }
    })?;
    let target_offset = extract_json_value(&full_text, "target_offset").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'target_offset' in loudnorm output".into(),
        }
    })?;

    Ok(LoudnormMeasurements {
        input_i,
        input_tp,
        input_lra,
        input_thresh,
        target_offset,
    })
}

/// Extract a numeric value from loudnorm JSON output for a given key.
///
/// Handles the format: `"key" : "value"` where value may be a number string.
fn extract_json_value(text: &str, key: &str) -> Option<f64> {
    let search = format!("\"{key}\"");
    let pos = text.find(&search)?;
    let after_key = &text[pos + search.len()..];

    // Skip whitespace and colon
    let after_colon = after_key.find(':')? + 1;
    let value_start = &after_key[after_colon..];

    // Find the quoted value
    let quote_start = value_start.find('"')? + 1;
    let value_after_quote = &value_start[quote_start..];
    let quote_end = value_after_quote.find('"')?;
    let value_str = &value_after_quote[..quote_end];

    value_str.trim().parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_audio_encoder_for_container() {
        assert_eq!(select_audio_encoder_for_container("mp4"), "aac");
        assert_eq!(select_audio_encoder_for_container("m4a"), "aac");
        assert_eq!(select_audio_encoder_for_container("mov"), "aac");
        assert_eq!(select_audio_encoder_for_container("webm"), "libopus");
        assert_eq!(select_audio_encoder_for_container("mkv"), "libopus");
        assert_eq!(select_audio_encoder_for_container("avi"), "libmp3lame");
        assert_eq!(select_audio_encoder_for_container("mp3"), "libmp3lame");
        assert_eq!(select_audio_encoder_for_container("flac"), "flac");
        assert_eq!(select_audio_encoder_for_container("wav"), "pcm_s16le");
        assert_eq!(select_audio_encoder_for_container("ts"), "aac");
        assert_eq!(select_audio_encoder_for_container("ogg"), "libopus");
        assert_eq!(select_audio_encoder_for_container("flv"), "aac");
        assert_eq!(select_audio_encoder_for_container("xyz"), "aac");
    }

    #[test]
    fn test_default_bitrate_for_encoder() {
        assert_eq!(default_bitrate_for_encoder("aac"), 128_000);
        assert_eq!(default_bitrate_for_encoder("libmp3lame"), 192_000);
        assert_eq!(default_bitrate_for_encoder("libopus"), 128_000);
        assert_eq!(default_bitrate_for_encoder("flac"), 0);
        assert_eq!(default_bitrate_for_encoder("pcm_s16le"), 0);
    }

    #[test]
    fn test_parse_loudnorm_json() {
        let lines = vec![
            "[Parsed_loudnorm_0 @ 0x...] ".to_string(),
            "{\n".to_string(),
            "    \"input_i\" : \"-24.50\",\n".to_string(),
            "    \"input_tp\" : \"-3.20\",\n".to_string(),
            "    \"input_lra\" : \"8.30\",\n".to_string(),
            "    \"input_thresh\" : \"-35.10\",\n".to_string(),
            "    \"output_i\" : \"-16.00\",\n".to_string(),
            "    \"output_tp\" : \"-1.50\",\n".to_string(),
            "    \"output_lra\" : \"7.20\",\n".to_string(),
            "    \"output_thresh\" : \"-26.60\",\n".to_string(),
            "    \"normalization_type\" : \"dynamic\",\n".to_string(),
            "    \"target_offset\" : \"0.50\"\n".to_string(),
            "}\n".to_string(),
        ];

        let m = parse_loudnorm_json(&lines).unwrap();
        assert!((m.input_i - (-24.5)).abs() < 0.01);
        assert!((m.input_tp - (-3.2)).abs() < 0.01);
        assert!((m.input_lra - 8.3).abs() < 0.01);
        assert!((m.input_thresh - (-35.1)).abs() < 0.01);
        assert!((m.target_offset - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_parse_loudnorm_json_missing_field() {
        let lines = vec!["{ \"input_i\" : \"-24.50\", \"input_tp\" : \"-3.20\" }".to_string()];

        let result = parse_loudnorm_json(&lines);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_json_value() {
        let text = r#""input_i" : "-24.50""#;
        assert!((extract_json_value(text, "input_i").unwrap() - (-24.5)).abs() < 0.01);

        let text = r#""target_offset" : "0.50""#;
        assert!((extract_json_value(text, "target_offset").unwrap() - 0.5).abs() < 0.01);

        assert!(extract_json_value(text, "nonexistent").is_none());
    }

    #[test]
    fn test_build_alimiter_spec_headroom() {
        // target_tp=-1.0 → ceiling = 10^((-1.0 - 1.5) / 20) = 10^(-0.125)
        let spec = build_alimiter_spec(-1.0);
        assert!(spec.starts_with("alimiter=limit="));
        assert!(spec.contains("attack=5"));
        assert!(spec.contains("release=50"));

        // Verify headroom: ceiling should be lower than 10^(-1/20) ≈ 0.891
        // With 1.5 dB headroom: 10^(-2.5/20) ≈ 0.750
        let limit_str = spec
            .strip_prefix("alimiter=limit=")
            .unwrap()
            .split(':')
            .next()
            .unwrap();
        let limit: f64 = limit_str.parse().unwrap();
        let expected = 10f64.powf((-1.0 - ALIMITER_TP_HEADROOM_DB) / 20.0);
        assert!(
            (limit - expected).abs() < 0.001,
            "limit={limit}, expected={expected}"
        );
    }

    #[test]
    fn test_build_loudnorm_pass2_filter_linear_no_shortfall() {
        // Source I=-20, TP=-7 → target I=-14, TP=-1
        // shortfall=0 → always linear=true (alimiter added by caller)
        let opts = NormalizeOptions {
            mode: AudioNormMode::Loudnorm,
            target_i: -14.0,
            target_tp: -1.0,
            target_lra: 11.0,
            ..Default::default()
        };
        let m = LoudnormMeasurements {
            input_i: -20.0,
            input_tp: -7.0,
            input_lra: 8.0,
            input_thresh: -30.0,
            target_offset: 0.0,
        };
        let filter = build_loudnorm_pass2_filter(&opts, &m);
        assert!(filter.contains("linear=true"));
        // alimiter is no longer part of this function (caller appends it)
        assert!(!filter.contains("alimiter="));
        assert!(!filter.contains("volume="));
        assert!(!filter.contains("acompressor="));
    }

    #[test]
    fn test_build_loudnorm_pass2_filter_moderate_shortfall() {
        // Source I=-17, TP=-1.5 → target I=-14, TP=-1 → shortfall=2.5
        // V2: still uses linear=true (no volume boost tier)
        let opts = NormalizeOptions {
            mode: AudioNormMode::Loudnorm,
            target_i: -14.0,
            target_tp: -1.0,
            target_lra: 11.0,
            ..Default::default()
        };
        let m = LoudnormMeasurements {
            input_i: -17.0,
            input_tp: -1.5,
            input_lra: 8.0,
            input_thresh: -27.0,
            target_offset: 0.0,
        };
        let filter = build_loudnorm_pass2_filter(&opts, &m);
        assert!(filter.contains("linear=true"));
        assert!(!filter.contains("alimiter="));
        assert!(!filter.contains("volume="));
    }

    #[test]
    fn test_build_loudnorm_pass2_filter_large_shortfall() {
        // Source I=-30, TP=-1 → target I=-14, TP=-1 → shortfall=16
        // V2: still uses linear=true (loudnorm falls back to dynamic internally)
        let opts = NormalizeOptions {
            mode: AudioNormMode::Loudnorm,
            target_i: -14.0,
            target_tp: -1.0,
            target_lra: 11.0,
            ..Default::default()
        };
        let m = LoudnormMeasurements {
            input_i: -30.0,
            input_tp: -1.0,
            input_lra: 12.0,
            input_thresh: -40.0,
            target_offset: 0.0,
        };
        let filter = build_loudnorm_pass2_filter(&opts, &m);
        assert!(filter.contains("linear=true"));
        assert!(!filter.contains("alimiter="));
        assert!(!filter.contains("linear=false"));
    }

    #[test]
    fn test_build_loudnorm_pass2_filter_force_dynamic() {
        let opts = NormalizeOptions {
            mode: AudioNormMode::Loudnorm,
            target_i: -14.0,
            target_tp: -1.0,
            target_lra: 11.0,
            force_dynamic: true,
            ..Default::default()
        };
        let m = LoudnormMeasurements {
            input_i: -20.0,
            input_tp: -7.0,
            input_lra: 8.0,
            input_thresh: -30.0,
            target_offset: 0.0,
        };
        let filter = build_loudnorm_pass2_filter(&opts, &m);
        assert!(filter.contains("linear=false"));
        assert!(!filter.contains("alimiter="));
        assert!(!filter.contains("linear=true"));
    }

    #[test]
    fn test_build_loudnorm_pass2_filter_precompress() {
        let opts = NormalizeOptions {
            mode: AudioNormMode::Loudnorm,
            target_i: -14.0,
            target_tp: -1.0,
            target_lra: 11.0,
            precompress: true,
            ..Default::default()
        };
        let m = LoudnormMeasurements {
            input_i: -20.0,
            input_tp: -7.0,
            input_lra: 8.0,
            input_thresh: -30.0,
            target_offset: 0.0,
        };
        let filter = build_loudnorm_pass2_filter(&opts, &m);
        assert!(filter.contains("acompressor="));
        assert!(filter.contains("linear=true"));
        assert!(!filter.contains("alimiter="));
        // acompressor should come BEFORE loudnorm
        let comp_pos = filter.find("acompressor=").unwrap();
        let loud_pos = filter.find("loudnorm=").unwrap();
        assert!(comp_pos < loud_pos, "acompressor must precede loudnorm");
    }

    #[test]
    fn test_audio_only_extension_for() {
        // MOV-based formats now use MKA to avoid ENOMEM
        assert_eq!(audio_only_extension_for("mp4"), "mka");
        assert_eq!(audio_only_extension_for("m4a"), "mka");
        assert_eq!(audio_only_extension_for("mov"), "mka");
        assert_eq!(audio_only_extension_for("f4v"), "mka");
        assert_eq!(audio_only_extension_for("3gp"), "mka");
        assert_eq!(audio_only_extension_for("ts"), "mka");
        assert_eq!(audio_only_extension_for("mpg"), "mka");
        assert_eq!(audio_only_extension_for("flv"), "mka");

        // Matroska-based formats
        assert_eq!(audio_only_extension_for("mkv"), "mka");
        assert_eq!(audio_only_extension_for("mka"), "mka");
        assert_eq!(audio_only_extension_for("webm"), "mka");

        // Other formats
        assert_eq!(audio_only_extension_for("avi"), "mp3");
        assert_eq!(audio_only_extension_for("mp3"), "mp3");
        assert_eq!(audio_only_extension_for("ogg"), "opus");
        assert_eq!(audio_only_extension_for("opus"), "opus");
        assert_eq!(audio_only_extension_for("flac"), "flac");
        assert_eq!(audio_only_extension_for("wav"), "wav");

        // Default
        assert_eq!(audio_only_extension_for("xyz"), "mka");
    }
}
