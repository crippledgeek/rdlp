//! Unsafe FFI helper functions.
//!
//! These encapsulate all `unsafe` FFI operations that lack safe wrappers
//! in `ffmpeg-the-third`, providing safe call-site signatures. The `unsafe`
//! blocks are limited to these well-documented helpers.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` codec parameter types use `u32`/`i32`. Conversions
//!   are audited and within valid ranges.
//! - `clippy::redundant_pub_crate`: `pub(crate)` functions are accessed from sibling
//!   modules via `crate::ffmpeg::ffi_helpers::*`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::redundant_pub_crate,
    clippy::similar_names,  // FFI convention: ctx_ptr / dec_ctx / enc_ctx are standard names
)]

mod filter_graph;

use crate::error::Result;

use super::FFmpegRunner;

/// What to do with a stream's copied codec tag for the target container.
///
/// Decided once, by [`FFmpegRunner::resolve_codec_tag`], and applied by
/// [`FFmpegRunner::add_stream_copy`] — the single call site for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodecTagAction {
    /// The muxer's codec-tag table has an entry for this codec: keep the
    /// tag `avcodec_parameters_copy` already copied from the source stream.
    Preserve,
    /// The muxer isn't tag-driven (no codec-tag table at all, e.g.
    /// MPEG-TS): zeroing has no effect either way; zero for consistency
    /// with this crate's historical behavior.
    Clear,
}

impl FFmpegRunner {
    /// Add a stream-copy output stream: add stream, copy parameters, resolve
    /// the codec tag for the target container.
    ///
    /// This is the standard pattern for remuxing/metadata/merge where a stream
    /// is copied without re-encoding. Returns the output stream index.
    ///
    /// # Arguments
    /// * `octx` - Output format context
    /// * `params` - Input stream parameters to copy
    /// * `context_msg` - Error context message (e.g., "for remux", "for merge video")
    ///
    /// # Errors
    ///
    /// Returns an error if the target container's muxer cannot represent the
    /// stream's codec at all (see [`Self::resolve_codec_tag`]).
    pub(crate) fn add_stream_copy(
        octx: &mut ffmpeg_the_third::format::context::Output,
        params: impl ffmpeg_the_third::AsPtr<ffmpeg_the_third::ffi::AVCodecParameters>,
        context_msg: &str,
    ) -> Result<usize> {
        use crate::error::PostProcessError;
        use anyhow::Context;

        // Captured before `add_stream` takes a mutable borrow of `octx`.
        // SAFETY: `octx` owns a valid AVFormatContext; `oformat` is set once
        // at context-alloc time (`avformat_alloc_output_context2`) and never
        // changes afterward, so reading it through a short-lived reborrow
        // here is sound regardless of what happens to `octx` later.
        let oformat: *const ffmpeg_the_third::ffi::AVOutputFormat =
            unsafe { (*octx.as_mut_ptr()).oformat };

        let mut ost = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(PostProcessError::from)
            .context(format!("failed to add output stream {context_msg}"))?;
        ost.set_parameters(params);

        let params_ptr = ost.parameters().as_ptr();
        match Self::resolve_codec_tag(oformat, params_ptr)? {
            CodecTagAction::Preserve => {}
            CodecTagAction::Clear => Self::clear_codec_tag(params_ptr),
        }

        Ok(ost.index())
    }

    /// Decide what to do with a stream's codec tag for the target container
    /// (see [`CodecTagAction`]) — the single decision point `add_stream_copy`
    /// consults; no other call site re-implements this predicate.
    ///
    /// Uses `av_codec_get_tag`/`av_codec_get_id` — the same public `FFmpeg`
    /// API the muxer's own header-validation path uses internally — against
    /// `oformat.codec_tag`, since `ffmpeg-the-third` has no safe wrapper for
    /// either. (`avformat_query_codec` was tried first and rejected: it only
    /// asks "does this muxer support this `codec_id` at all", not "does it
    /// accept *this specific* tag" — reporting `1` for e.g. AAC-in-AVI even
    /// though the source's `mp4a` tag isn't one the AVI table recognizes,
    /// which broke real audio/FLV streams in this fix's own test matrix.)
    ///
    /// - The muxer has no codec-tag table at all (`oformat.codec_tag` is
    ///   null, e.g. MPEG-TS): tag-clearing is inert either way ->
    ///   [`CodecTagAction::Clear`].
    /// - The table has no entry for this codec's `codec_id` whatsoever
    ///   (`av_codec_get_tag` returns 0): the pairing cannot be represented in
    ///   this container by this `FFmpeg` build (a convention gap, not a
    ///   documented standard — no spec covers HEVC-in-AVI). Reject before
    ///   writing rather than let a zeroed tag silently decode as something
    ///   else (AVI + HEVC: tag stays 0, `riff.c` maps a zero fourcc to raw
    ///   video — exit 0, corrupt file).
    /// - The table has an entry for `codec_id`, and the source's specific
    ///   tag maps back to that same `codec_id` (`av_codec_get_id` round-trips)
    ///   -> [`CodecTagAction::Preserve`]. Zeroing here would make `FFmpeg`
    ///   auto-fill the tag from the muxer's own table, which for AVI
    ///   substitutes a literal `'H264'` fourcc — that value arms
    ///   `avienc.c`'s start-code guard and hard-fails AVCC-packaged H.264
    ///   (no start codes in the bitstream).
    /// - The table has an entry for `codec_id`, but under a *different* tag
    ///   than the source's -> [`CodecTagAction::Clear`]. Preserving would
    ///   write a tag the muxer's own validation rejects outright (observed
    ///   for AAC's `mp4a` into AVI, and H.264's `avc1` into FLV); zeroing
    ///   lets `FFmpeg` auto-fill the muxer's own preferred tag instead.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::PostProcessError::IncompatibleContainerCodec`]
    /// when the muxer's tag table has no entry for the stream's codec.
    fn resolve_codec_tag(
        oformat: *const ffmpeg_the_third::ffi::AVOutputFormat,
        params_ptr: *const ffmpeg_the_third::ffi::AVCodecParameters,
    ) -> Result<CodecTagAction> {
        use crate::error::PostProcessError;

        // SAFETY: `oformat` is the live output context's format descriptor
        // (see the SAFETY note at the `add_stream_copy` call site);
        // `params_ptr` is the just-copied codecpar of the stream we added to
        // that same context, so both reads are into live FFmpeg-owned memory.
        let (codec_id, source_tag, codec_type, tags) = unsafe {
            (
                (*params_ptr).codec_id,
                (*params_ptr).codec_tag,
                (*params_ptr).codec_type,
                (*oformat).codec_tag,
            )
        };

        if tags.is_null() {
            return Ok(CodecTagAction::Clear);
        }

        // SAFETY: `tags` was just proven non-null; both lookups are pure
        // reads over the muxer's static, compiled-in tag table.
        let (muxer_tag_for_codec, id_for_source_tag) = unsafe {
            (
                ffmpeg_the_third::ffi::av_codec_get_tag(tags, codec_id),
                ffmpeg_the_third::ffi::av_codec_get_id(tags, source_tag),
            )
        };

        if muxer_tag_for_codec != 0 {
            return Ok(if id_for_source_tag == codec_id {
                // The source's specific tag round-trips to this codec: keep it.
                CodecTagAction::Preserve
            } else {
                // Codec supported, but not under the source's specific tag;
                // let FFmpeg auto-fill its own preferred tag instead.
                CodecTagAction::Clear
            });
        }

        // No entry for this codec at all, under any tag: unrepresentable.
        // SAFETY: every registered muxer descriptor has a non-null,
        // NUL-terminated `name`.
        let container = unsafe {
            std::ffi::CStr::from_ptr((*oformat).name)
                .to_string_lossy()
                .into_owned()
        };
        let codec = ffmpeg_the_third::codec::Id::from(codec_id)
            .name()
            .to_string();
        let medium = match codec_type {
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO => "video",
            ffmpeg_the_third::ffi::AVMediaType::AVMEDIA_TYPE_AUDIO => "audio",
            _ => "stream",
        }
        .to_string();

        Err(PostProcessError::IncompatibleContainerCodec {
            container,
            codec,
            medium,
        })
    }

    /// Reset the codec tag to 0 for container compatibility.
    ///
    /// Only correct to call when the target muxer isn't tag-driven at all
    /// (see [`Self::resolve_codec_tag`]) — zeroing then has no effect, since
    /// there is no table for `FFmpeg` to auto-fill a tag from.
    pub(crate) fn clear_codec_tag(params_ptr: *const ffmpeg_the_third::ffi::AVCodecParameters) {
        // SAFETY: `params_ptr` points to a valid AVCodecParameters allocated by FFmpeg.
        // Setting codec_tag to 0 is always valid — it tells FFmpeg to auto-select.
        unsafe {
            (*params_ptr.cast_mut()).codec_tag = 0;
        }
    }

    /// Copy encoder parameters back to an output stream.
    ///
    /// After opening an encoder, its parameters (codec, dimensions, sample rate,
    /// etc.) must be copied to the corresponding output stream before writing
    /// the header.
    pub(crate) fn copy_encoder_params_to_stream(
        octx: &mut ffmpeg_the_third::format::context::Output,
        stream_index: usize,
        encoder_ptr: *const ffmpeg_the_third::ffi::AVCodecContext,
    ) {
        // SAFETY: `octx` owns the output context with a valid stream array.
        // `stream_index` was obtained from a stream added to this context.
        // `encoder_ptr` points to a valid, opened encoder context.
        unsafe {
            let stream_ptr = *(*octx.as_mut_ptr()).streams.add(stream_index);
            ffmpeg_the_third::ffi::avcodec_parameters_from_context(
                (*stream_ptr).codecpar,
                encoder_ptr,
            );
        }
    }

    /// Set the default channel layout for the given number of channels.
    pub(crate) fn set_default_channel_layout(
        encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext,
        channels: i32,
    ) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // `av_channel_layout_default` populates the ch_layout field in-place.
        unsafe {
            ffmpeg_the_third::ffi::av_channel_layout_default(
                &raw mut (*encoder_ptr).ch_layout,
                channels,
            );
        }
    }

    /// Pick a sample rate supported by the encoder codec.
    ///
    /// If the codec accepts any rate (`supported_samplerates` is NULL), returns
    /// `preferred` unchanged.  Otherwise returns `preferred` if it appears in
    /// the supported list, or the nearest supported rate (preferring higher
    /// rates on ties, which naturally selects 48 kHz for a 44.1 kHz source on
    /// libopus).
    pub(crate) fn pick_audio_sample_rate(codec: &ffmpeg_the_third::Codec, preferred: u32) -> u32 {
        // SAFETY: `codec.as_ptr()` returns a valid AVCodec pointer.
        // `supported_samplerates` is a NULL-terminated i32 array (or NULL if
        // the codec accepts any rate).
        unsafe {
            let ptr = codec.as_ptr();
            let rates = (*ptr).supported_samplerates;
            if rates.is_null() {
                return preferred;
            }

            let mut i: isize = 0;
            let mut best: Option<u32> = None;
            let mut best_dist = u32::MAX;
            loop {
                debug_assert!(
                    i < 1000,
                    "FFmpeg rate array iteration exceeded safety limit"
                );
                let rate = *rates.offset(i);
                if rate == 0 {
                    break;
                }
                let rate_u = rate as u32;
                if rate_u == preferred {
                    return preferred;
                }
                let dist = rate_u.abs_diff(preferred);
                if dist < best_dist || (dist == best_dist && rate_u > best.unwrap_or(0)) {
                    best = Some(rate_u);
                    best_dist = dist;
                }
                i += 1;
            }

            best.unwrap_or(preferred)
        }
    }

    /// Enable VBR (variable bitrate) quality mode on an encoder.
    pub(crate) fn set_vbr_quality(
        encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext,
        quality: i32,
    ) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // Setting QSCALE flag + global_quality is the standard way to enable VBR.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_QSCALE as i32;
            (*encoder_ptr).global_quality = quality * ffmpeg_the_third::ffi::FF_QP2LAMBDA;
        }
    }

    /// Set the global header flag on an encoder.
    ///
    /// Required when the output format needs codec parameters in the container
    /// header rather than in each packet (e.g., MP4, MKV).
    pub(crate) fn set_global_header_flag(encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // This flag is required by certain container formats.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    /// Write thumbnail packets from the thumbnail input to the output context.
    pub(crate) fn write_thumbnail_packets(
        thumb_ictx: &mut ffmpeg_the_third::format::context::Input,
        octx: &mut ffmpeg_the_third::format::context::Output,
        thumb_ist_index: usize,
        thumb_ist_time_base: ffmpeg_the_third::Rational,
        thumb_ost_index: usize,
    ) -> Result<()> {
        use crate::error::PostProcessError;

        let thumb_ost_time_base = octx
            .stream(thumb_ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "thumbnail output stream {thumb_ost_index} not found"
                ))
            })?
            .time_base();
        for result in thumb_ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read thumbnail packet: {e}"),
                })?;
            if stream.index() == thumb_ist_index {
                packet.rescale_ts(thumb_ist_time_base, thumb_ost_time_base);
                packet.set_position(-1);
                packet.set_stream(thumb_ost_index);
                packet.write_interleaved(octx).map_err(|e| {
                    PostProcessError::FFmpegLibraryError {
                        message: format!("failed to write thumbnail packet: {e}"),
                    }
                })?;
            }
        }
        Ok(())
    }

    /// Set `AVDISCARD_ALL` on all non-audio streams in an input context.
    ///
    /// Tells the demuxer to skip non-audio packets entirely, avoiding
    /// memory allocation for large video packets during audio-only analysis.
    pub(crate) fn discard_non_audio_streams(
        ictx: &mut ffmpeg_the_third::format::context::Input,
        audio_stream_index: usize,
    ) {
        // SAFETY: `ictx` owns a valid AVFormatContext. Setting `discard` on
        // streams is a standard FFmpeg operation that tells the demuxer to
        // skip packets for those streams.
        unsafe {
            let ctx_ptr = ictx.as_mut_ptr();
            let nb_streams = (*ctx_ptr).nb_streams as usize;
            for i in 0..nb_streams {
                if i != audio_stream_index {
                    let stream = *(*ctx_ptr).streams.add(i);
                    (*stream).discard = ffmpeg_the_third::ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
        }
    }

    /// Set the frame size on an audio buffersink filter.
    ///
    /// Tells the buffersink to output exactly `frame_size` samples per frame.
    /// The last frame at EOF is automatically zero-padded. This is the proper
    /// way to feed fixed-frame-size encoders (AAC=1024, MP3=1152, Opus=960).
    ///
    /// No-op if `frame_size` is 0 (variable-frame-size codecs like FLAC/PCM).
    pub(crate) fn set_buffersink_frame_size(
        graph: &mut ffmpeg_the_third::filter::Graph,
        sink_name: &str,
        frame_size: u32,
    ) {
        if frame_size == 0 {
            return;
        }

        let Ok(name_c) = std::ffi::CString::new(sink_name) else {
            return;
        };

        // SAFETY: `graph` owns a valid filter graph. `avfilter_graph_get_filter`
        // retrieves the named context. `av_buffersink_set_frame_size` sets the
        // min/max sample counts on the sink's input link.
        unsafe {
            let ctx = ffmpeg_the_third::ffi::avfilter_graph_get_filter(
                graph.as_mut_ptr(),
                name_c.as_ptr(),
            );
            if !ctx.is_null() {
                ffmpeg_the_third::ffi::av_buffersink_set_frame_size(ctx, frame_size);
            }
        }
    }

    /// Configure a stream with `ATTACHED_PIC` disposition (for cover art).
    ///
    /// Sets the stream disposition and clears the codec tag. Used for MP4,
    /// FLAC, OGG, and other containers that embed cover art as a video stream
    /// with special disposition.
    pub(crate) fn set_attached_pic_disposition(stream_ptr: *mut ffmpeg_the_third::ffi::AVStream) {
        // SAFETY: `stream_ptr` is a valid output stream pointer from a live
        // output context. Setting disposition and clearing codec_tag configures
        // the stream as cover art.
        unsafe {
            (*stream_ptr).disposition = ffmpeg_the_third::ffi::AV_DISPOSITION_ATTACHED_PIC;
            (*((*stream_ptr).codecpar)).codec_tag = 0;
        }
    }
}

/// Release packet buffer references. Idempotent — safe on empty packets.
///
/// SAFETY: `Packet::as_ptr()` returns a valid, non-null `AVPacket` pointer
/// owned by the Rust wrapper. `av_packet_unref` only zeroes internal fields.
pub fn packet_unref(pkt: &mut ffmpeg_the_third::Packet) {
    use ffmpeg_the_third::packet::Mut;
    unsafe { ffmpeg_the_third::ffi::av_packet_unref(pkt.as_mut_ptr()) }
}

/// Release audio frame buffer references. Idempotent — safe on empty frames.
///
/// Calling this after `filter.source().add(&frame)` and after
/// `encoder.send_frame(&frame)` releases our reference immediately,
/// reducing peak memory when the filter/encoder also holds a ref.
pub fn frame_unref_audio(frame: &mut ffmpeg_the_third::frame::Audio) {
    unsafe { ffmpeg_the_third::ffi::av_frame_unref((*frame).as_mut_ptr()) }
}

/// Release video frame buffer references. Idempotent — safe on empty frames.
pub fn frame_unref_video(frame: &mut ffmpeg_the_third::frame::Video) {
    unsafe { ffmpeg_the_third::ffi::av_frame_unref((*frame).as_mut_ptr()) }
}

/// Force single-threaded operation on a codec context.
///
/// Must be called **before** `avcodec_open2` (i.e. before `.audio()?` or
/// `.open_as()`). Setting `thread_count = 1` causes `FFmpeg`'s
/// `validate_thread_parameters()` to set `active_thread_type = 0`, which
/// disables both frame threading and slice threading. This eliminates:
/// - Frame threading's per-thread decode buffer pre-allocation (N × `frame_size`)
/// - Slice threading's per-slice scratch buffers
///
/// For audio normalization paths this is the primary RSS reduction knob:
/// the default auto-threading can allocate hundreds of MB in decode/encode
/// buffers that are unnecessary for a single-stream sequential pipeline.
///
/// # Safety
///
/// `ctx_ptr` must point to a valid, **unopened** `AVCodecContext`.
pub fn set_single_thread_codec(ctx_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext) {
    // SAFETY: caller guarantees ctx_ptr is valid and unopened.
    // Setting thread_count before open is the documented way to control threading.
    unsafe {
        (*ctx_ptr).thread_count = 1;
    }
}

/// Read `thread_count` and `active_thread_type` from an opened codec context.
///
/// Returns `(thread_count, active_thread_type)` for diagnostic logging.
///
/// # Safety
///
/// `ctx_ptr` must point to a valid, opened `AVCodecContext`.
pub const fn codec_threading_info(
    ctx_ptr: *const ffmpeg_the_third::ffi::AVCodecContext,
) -> (i32, i32) {
    unsafe { ((*ctx_ptr).thread_count, (*ctx_ptr).active_thread_type) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_unref_on_empty_packet() {
        let mut pkt = ffmpeg_the_third::Packet::empty();
        // Should not panic on an empty/zeroed packet
        packet_unref(&mut pkt);
        // Double-unref should also be safe (idempotent)
        packet_unref(&mut pkt);
    }

    #[test]
    fn frame_unref_audio_on_empty_frame() {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        frame_unref_audio(&mut frame);
        frame_unref_audio(&mut frame);
    }

    #[test]
    fn frame_unref_video_on_empty_frame() {
        let mut frame = ffmpeg_the_third::frame::Video::empty();
        frame_unref_video(&mut frame);
        frame_unref_video(&mut frame);
    }

    #[test]
    fn pick_sample_rate_opus_rejects_44100() {
        crate::ffmpeg::ensure_init().unwrap();
        let codec = ffmpeg_the_third::encoder::find_by_name("libopus").unwrap();
        // 44100 is not a supported libopus rate; should pick 48000 (nearest).
        let rate = FFmpegRunner::pick_audio_sample_rate(&codec, 44100);
        assert_eq!(rate, 48000, "libopus should resample 44100→48000");
    }

    #[test]
    fn pick_sample_rate_opus_accepts_48000() {
        crate::ffmpeg::ensure_init().unwrap();
        let codec = ffmpeg_the_third::encoder::find_by_name("libopus").unwrap();
        let rate = FFmpegRunner::pick_audio_sample_rate(&codec, 48000);
        assert_eq!(rate, 48000);
    }

    #[test]
    fn pick_sample_rate_aac_accepts_44100() {
        crate::ffmpeg::ensure_init().unwrap();
        if let Some(codec) = ffmpeg_the_third::encoder::find_by_name("aac") {
            // AAC supports 44100; should return it unchanged.
            let rate = FFmpegRunner::pick_audio_sample_rate(&codec, 44100);
            assert_eq!(rate, 44100);
        }
    }
}
