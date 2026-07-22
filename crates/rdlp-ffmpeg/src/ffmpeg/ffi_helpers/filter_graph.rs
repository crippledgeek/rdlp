//! Filter graph construction and validation via raw FFI.
//!
//! Provides `add_abuffer_to_graph` and `parse_and_validate_filter_graph`
//! which bypass `ffmpeg-the-third` wrapper limitations for audio filter
//! graph setup, plus the two audio-graph builders every route goes through:
//! [`build_audio_filter_graph`] (any filter spec) and
//! [`build_encoder_adapted_audio_filter`] (a spec derived from an encoder).
//!
//! # Lint allowances
//!
//! - `clippy::expect_used`: `CString::new(format_args)` calls in this module use
//!   `format!`-produced strings that are guaranteed NUL-free (they contain only
//!   decimal digits, commas, and codec names). Any bug would surface at first test run.
//! - `clippy::redundant_pub_crate`: `pub(crate)` methods in this `impl FFmpegRunner`
//!   block are accessed from normalize and transcode submodules via `crate::` paths.

#![allow(clippy::expect_used, clippy::redundant_pub_crate)]

use std::ffi::CString;

use crate::error::{PostProcessError, Result};

use super::super::FFmpegRunner;

/// What the graph's sink end must produce.
///
/// The filter chain and the sink's frame size are one decision, not two:
/// `frame_size` is meaningless without knowing which encoder the chain feeds,
/// and a chain built for a fixed-frame-size encoder is unusable without it.
/// Grouping them is what makes [`build_audio_filter_graph`] apply both — the
/// previous shape let a caller build the graph and simply forget the frame
/// size, which is exactly how #638 shipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioSinkSpec<'a> {
    /// Filter chain between `abuffer` and `abuffersink` (e.g. an `aformat`).
    pub filter_spec: &'a str,
    /// Samples per frame the sink must emit, from the **opened** encoder's
    /// `frame_size`. `0` means the consumer accepts variable-length frames
    /// (PCM, or a measurement-only graph with no encoder at all) and leaves
    /// the sink's chunking alone.
    pub frame_size: u32,
}

/// Build `abuffer → {sink.filter_spec} → abuffersink` for `decoder`'s output.
///
/// This is the single audio-graph constructor for the whole crate — normalize,
/// recode, and extract all route through it — and it applies
/// `sink.frame_size` itself. Most audio encoders reject a non-final frame of
/// any other length with a bare `EINVAL` from `avcodec_send_frame`, so pairing
/// the two here rather than trusting each caller to remember a follow-up call
/// is the structural form of the #638 fix.
///
/// # Errors
///
/// Returns an error if a filter is missing from the build, or the graph fails
/// to parse, configure, or validate.
pub(crate) fn build_audio_filter_graph(
    decoder: &ffmpeg_the_third::decoder::Audio,
    input_time_base: ffmpeg_the_third::Rational,
    sink: AudioSinkSpec<'_>,
) -> Result<ffmpeg_the_third::filter::Graph> {
    let mut graph = ffmpeg_the_third::filter::Graph::new();

    let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
        .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

    FFmpegRunner::add_abuffer_to_graph(
        &mut graph,
        "in",
        input_time_base,
        decoder.rate(),
        decoder.format().name(),
        &decoder.ch_layout().description(),
    )?;
    graph
        .add(&abuffersink, "out", "")
        .map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to add abuffersink filter: {e}"),
        })?;

    FFmpegRunner::parse_and_validate_filter_graph(&mut graph, "in", "out", sink.filter_spec)?;

    // After `avfilter_graph_config`, matching FFmpeg's own ordering in
    // `ffmpeg_filter.c`: the sink's input link must be configured before its
    // frame size can be constrained.
    FFmpegRunner::set_buffersink_frame_size(&mut graph, "out", sink.frame_size);

    Ok(graph)
}

/// Build a graph that adapts `decoder`'s frames to everything `encoder` needs.
///
/// Two adaptations are required, and an encoder rejects the frame with a bare
/// `EINVAL` if either is missing:
///
/// 1. **Sample format / rate / channel layout**, via `aformat`. Format
///    negotiation inserts the resampler itself — `auto_convert` is left at its
///    `AVFILTER_AUTO_CONVERT_ALL` default — so no explicit `aresample` stage is
///    needed.
/// 2. **Frame size**, applied by [`build_audio_filter_graph`].
///
/// `encoder` must be **opened**: `frame_size` is only populated by
/// `avcodec_open2`, which is why this takes `encoder::audio::Encoder` rather
/// than the pre-open `encoder::audio::Audio`. Taking the opened type makes
/// "adapt before the encoder is ready" a compile error instead of a silent
/// zero.
///
/// Both transcode routes — standalone `extract_audio` and the audio side of a
/// video recode — call this. They had separate builders until #638, and only
/// the recode one set the frame size, so `--extract-audio` worked for AAC
/// alone: the one target whose frame size already matched the AAC decoder's
/// 1024-sample output.
///
/// # Errors
///
/// Propagates any failure from [`build_audio_filter_graph`].
pub(crate) fn build_encoder_adapted_audio_filter(
    decoder: &ffmpeg_the_third::decoder::Audio,
    encoder: &ffmpeg_the_third::encoder::audio::Encoder,
    input_time_base: ffmpeg_the_third::Rational,
) -> Result<ffmpeg_the_third::filter::Graph> {
    // The channel layout comes from the encoder's own `ch_layout` (configured
    // from the decoder's channel count before open), so layouts beyond
    // mono/stereo are described correctly. The builder this replaced hardcoded
    // `"stereo"` for anything above 1 channel and aborted every 5.1 recode
    // with "Changing audio frame properties on the fly is not supported".
    let filter_spec = format!(
        "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
        encoder.format().name(),
        encoder.rate(),
        encoder.ch_layout().description(),
    );

    build_audio_filter_graph(
        decoder,
        input_time_base,
        AudioSinkSpec {
            filter_spec: &filter_spec,
            frame_size: encoder.frame_size(),
        },
    )
}

impl FFmpegRunner {
    /// Add an `abuffer` audio source filter to a graph using raw FFI.
    ///
    /// Uses `avfilter_graph_alloc_filter` + `av_opt_set*` + `avfilter_init_str`
    /// instead of the args-string approach via `Graph::add()`. Required because
    /// `FFmpeg` 8.0's abuffer option is `"channel_layout"` (not `"chlayout"`),
    /// and the args-string parser rejects unknown option names.
    pub(crate) fn add_abuffer_to_graph(
        graph: &mut ffmpeg_the_third::filter::Graph,
        name: &str,
        time_base: ffmpeg_the_third::Rational,
        sample_rate: u32,
        sample_fmt_name: &str,
        channel_layout_desc: &str,
    ) -> Result<()> {
        let abuffer = ffmpeg_the_third::filter::find("abuffer")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffer filter not found"))?;

        let name_c = CString::new(name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid filter name"))?;
        let ch_layout_c = CString::new(channel_layout_desc)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid channel layout"))?;
        let sample_fmt_c = CString::new(sample_fmt_name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid sample format name"))?;

        // Option key CStrings (static values, can't fail)
        let key_channel_layout =
            CString::new("channel_layout").expect("static string has no null bytes");
        let key_sample_fmt = CString::new("sample_fmt").expect("static string has no null bytes");
        let key_time_base = CString::new("time_base").expect("static string has no null bytes");
        let key_sample_rate = CString::new("sample_rate").expect("static string has no null bytes");

        // SAFETY: All pointers are valid for the duration of this block.
        // avfilter_graph_alloc_filter allocates within the graph's lifetime.
        // av_opt_set* write to the allocated filter context.
        // avfilter_init_str finalizes the filter initialization.
        unsafe {
            let ctx = ffmpeg_the_third::ffi::avfilter_graph_alloc_filter(
                graph.as_mut_ptr(),
                abuffer.as_ptr(),
                name_c.as_ptr(),
            );
            if ctx.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "failed to allocate abuffer filter context".into(),
                });
            }

            let search = ffmpeg_the_third::ffi::AV_OPT_SEARCH_CHILDREN;

            let ret = ffmpeg_the_third::ffi::av_opt_set(
                ctx.cast::<std::ffi::c_void>(),
                key_channel_layout.as_ptr(),
                ch_layout_c.as_ptr(),
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "av_opt_set channel_layout={channel_layout_desc} failed: {ret}"
                    ),
                });
            }

            let ret = ffmpeg_the_third::ffi::av_opt_set(
                ctx.cast::<std::ffi::c_void>(),
                key_sample_fmt.as_ptr(),
                sample_fmt_c.as_ptr(),
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("av_opt_set sample_fmt={sample_fmt_name} failed: {ret}"),
                });
            }

            let ret = ffmpeg_the_third::ffi::av_opt_set_q(
                ctx.cast::<std::ffi::c_void>(),
                key_time_base.as_ptr(),
                ffmpeg_the_third::ffi::AVRational {
                    num: time_base.numerator(),
                    den: time_base.denominator(),
                },
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "av_opt_set_q time_base={}/{} failed: {ret}",
                        time_base.numerator(),
                        time_base.denominator()
                    ),
                });
            }

            let ret = ffmpeg_the_third::ffi::av_opt_set_int(
                ctx.cast::<std::ffi::c_void>(),
                key_sample_rate.as_ptr(),
                i64::from(sample_rate),
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("av_opt_set_int sample_rate={sample_rate} failed: {ret}"),
                });
            }

            let ret = ffmpeg_the_third::ffi::avfilter_init_str(ctx, std::ptr::null());
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avfilter_init_str for abuffer '{name}' failed: {ret}"),
                });
            }
        }

        Ok(())
    }

    /// Parse a filter spec between named source/sink and validate the graph.
    ///
    /// Bypasses the `ffmpeg-the-third` wrapper's `Parser::parse()` which may
    /// swap the `inputs`/`outputs` parameters to `avfilter_graph_parse_ptr`.
    /// Instead calls FFI directly matching `FFmpeg`'s official `filter_audio.c`
    /// example: `outputs` = source (abuffer), `inputs` = sink (abuffersink).
    pub(crate) fn parse_and_validate_filter_graph(
        graph: &mut ffmpeg_the_third::filter::Graph,
        src_name: &str,
        sink_name: &str,
        filter_spec: &str,
    ) -> Result<()> {
        let src_name_c = CString::new(src_name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid source filter name"))?;
        let sink_name_c = CString::new(sink_name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid sink filter name"))?;
        let spec_c = CString::new(filter_spec)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid filter spec"))?;

        // SAFETY: All pointers are valid for the duration of this block.
        // avfilter_graph_get_filter retrieves contexts by name from the graph.
        // avfilter_inout_alloc + av_strdup allocate memory freed by avfilter_inout_free.
        // avfilter_graph_parse_ptr parses the spec and links intermediate filters.
        // avfilter_graph_config validates format negotiation and link configuration.
        unsafe {
            let src_ctx = ffmpeg_the_third::ffi::avfilter_graph_get_filter(
                graph.as_mut_ptr(),
                src_name_c.as_ptr(),
            );
            if src_ctx.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("filter '{src_name}' not found in graph"),
                });
            }

            let sink_ctx = ffmpeg_the_third::ffi::avfilter_graph_get_filter(
                graph.as_mut_ptr(),
                sink_name_c.as_ptr(),
            );
            if sink_ctx.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("filter '{sink_name}' not found in graph"),
                });
            }

            // `outputs` = source (abuffer) with unconnected output pad.
            // The parsed chain's implicit [in] label connects FROM this pad.
            let outputs = ffmpeg_the_third::ffi::avfilter_inout_alloc();
            if outputs.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "failed to allocate AVFilterInOut for outputs".into(),
                });
            }
            (*outputs).name = ffmpeg_the_third::ffi::av_strdup(src_name_c.as_ptr());
            (*outputs).filter_ctx = src_ctx;
            (*outputs).pad_idx = 0;
            (*outputs).next = std::ptr::null_mut();

            // `inputs` = sink (abuffersink) with unconnected input pad.
            // The parsed chain's implicit [out] label connects TO this pad.
            let inputs = ffmpeg_the_third::ffi::avfilter_inout_alloc();
            if inputs.is_null() {
                let mut out_ptr = outputs;
                ffmpeg_the_third::ffi::avfilter_inout_free(&raw mut out_ptr);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "failed to allocate AVFilterInOut for inputs".into(),
                });
            }
            (*inputs).name = ffmpeg_the_third::ffi::av_strdup(sink_name_c.as_ptr());
            (*inputs).filter_ctx = sink_ctx;
            (*inputs).pad_idx = 0;
            (*inputs).next = std::ptr::null_mut();

            // Parse spec with FFmpeg-standard parameter order:
            //   3rd = &inputs  (sink pads, abuffersink)
            //   4th = &outputs (source pads, abuffer)
            let mut inputs_ptr = inputs;
            let mut outputs_ptr = outputs;
            let ret = ffmpeg_the_third::ffi::avfilter_graph_parse_ptr(
                graph.as_mut_ptr(),
                spec_c.as_ptr(),
                &raw mut inputs_ptr,
                &raw mut outputs_ptr,
                std::ptr::null_mut(),
            );

            // Free InOut structures (parse_ptr may set consumed pointers to NULL)
            ffmpeg_the_third::ffi::avfilter_inout_free(&raw mut inputs_ptr);
            ffmpeg_the_third::ffi::avfilter_inout_free(&raw mut outputs_ptr);

            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "avfilter_graph_parse_ptr failed for spec '{filter_spec}': {ret}"
                    ),
                });
            }

            // Validate and configure the complete graph
            let ret = ffmpeg_the_third::ffi::avfilter_graph_config(
                graph.as_mut_ptr(),
                std::ptr::null_mut(),
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avfilter_graph_config failed: {ret}"),
                });
            }
        }

        Ok(())
    }
}
