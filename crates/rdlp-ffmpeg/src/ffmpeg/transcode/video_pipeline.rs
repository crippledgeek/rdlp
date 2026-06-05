//! Video transcoding pipeline helpers.
//!
//! Provides pixel format selection, video filter graph construction,
//! and frame/packet processing functions used by `convert_video_transcode_sync`.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` stream index types require `usize`/`i32` conversions.
//!   All casts are audited and within valid ranges.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

use log::warn;

use ffmpeg_the_third::packet::Ref as _;

use crate::error::{PostProcessError, Result};

use super::super::FFmpegRunner;
use super::super::ffi_helpers::{frame_unref_video, packet_unref};
use super::mux_timing::{av_strerror_string, diagnose_mux_io, get_process_rss_kb};

impl FFmpegRunner {
    /// Pick a pixel format supported by the video encoder, preferring the decoder's format.
    pub(super) fn pick_video_pixel_format(
        codec: &ffmpeg_the_third::Codec,
        preferred: ffmpeg_the_third::format::Pixel,
    ) -> ffmpeg_the_third::format::Pixel {
        unsafe {
            let ptr = codec.as_ptr();
            let pix_fmts = (*ptr).pix_fmts;
            if pix_fmts.is_null() {
                return preferred;
            }

            let mut i = 0;
            let mut first = None;
            loop {
                let fmt = *pix_fmts.offset(i);
                if fmt == ffmpeg_the_third::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                    break;
                }
                let pixel = ffmpeg_the_third::format::Pixel::from(fmt);
                first.get_or_insert(pixel);
                if pixel == preferred {
                    return preferred;
                }
                i += 1;
            }

            first.unwrap_or(preferred)
        }
    }

    /// Enforce a decodable pixel format for codecs whose decoder can't read
    /// back everything the encoder can write.
    ///
    /// libxavs2 (AVS2) can be driven at 10-bit, but **libdavs2 cannot decode
    /// 10-bit AVS2** (`Un-supported bit-depth 10`) — rdlp must never emit an
    /// AVS2 file it can't read back. The `libxavs2` wrapper also declares
    /// only `AV_PIX_FMT_YUV420P` (8-bit), so `pick_video_pixel_format` already
    /// returns 8-bit today; this makes the invariant explicit and regression-
    /// proof against a future pixel-format-selection change. Non-AVS2 codecs
    /// (e.g. libvvenc's 10-bit `yuv420p10`) pass through unchanged. (issue #332)
    pub(super) fn enforce_decodable_pixfmt(
        codec_name: &str,
        picked: ffmpeg_the_third::format::Pixel,
    ) -> ffmpeg_the_third::format::Pixel {
        if codec_name == "libxavs2" {
            ffmpeg_the_third::format::Pixel::YUV420P
        } else {
            picked
        }
    }

    /// Rescale a frame pts from `src` to `dst` `time_base`.
    ///
    /// Errors on a zero-denominator `time_base` instead of letting `av_rescale_q`
    /// return its `INT64_MIN` (`AV_NOPTS_VALUE`) sentinel, which would otherwise
    /// be fed to the encoder and surface as a confusing muxer error downstream.
    /// A zero-denominator tb only arises from a degenerate/corrupt container
    /// reaching the `video_ist_time_base` fallback. (issue #331)
    pub(super) fn rescale_frame_pts(
        pts: i64,
        src: ffmpeg_the_third::Rational,
        dst: ffmpeg_the_third::Rational,
    ) -> Result<i64> {
        if src.denominator() == 0 || dst.denominator() == 0 {
            return Err(PostProcessError::ffmpeg_failed(format!(
                "cannot rescale frame pts: zero-denominator time_base \
                 (src={}/{}, dst={}/{})",
                src.numerator(),
                src.denominator(),
                dst.numerator(),
                dst.denominator(),
            )));
        }
        let src_q = ffmpeg_the_third::ffi::AVRational {
            num: src.numerator(),
            den: src.denominator(),
        };
        let dst_q = ffmpeg_the_third::ffi::AVRational {
            num: dst.numerator(),
            den: dst.denominator(),
        };
        // SAFETY: av_rescale_q is a pure arithmetic helper over plain values
        // (no pointers/aliasing). Denominators are verified non-zero above, so
        // it cannot return the INT64_MIN zero-denominator sentinel.
        Ok(unsafe { ffmpeg_the_third::ffi::av_rescale_q(pts, src_q, dst_q) })
    }

    /// Build a video filter graph for pixel format conversion.
    ///
    /// Uses `buffer` -> `format` -> `buffersink` to convert pixel format
    /// from decoder output to encoder input format.
    pub(super) fn build_video_filter(
        decoder: &ffmpeg_the_third::decoder::Video,
        encoder: &ffmpeg_the_third::encoder::video::Video,
        ist_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let mut graph = ffmpeg_the_third::filter::Graph::new();

        let buffer = ffmpeg_the_third::filter::find("buffer")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("buffer filter not found"))?;
        let buffersink = ffmpeg_the_third::filter::find("buffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("buffersink filter not found"))?;

        // Pixel aspect ratio (default 1:1 if unknown)
        let sar = decoder.aspect_ratio();
        let sar_num = sar.numerator().max(1);
        let sar_den = sar.denominator().max(1);

        // pixel-format NAME, not `as i32` — ffmpeg-the-third's Pixel discriminant
        // != C AVPixelFormat value (YUV420P is 1 in Rust, 0 in C).
        let in_pix_fmt_name = decoder
            .format()
            .descriptor()
            .map_or_else(|| "yuv420p".to_string(), |d| d.name().to_string());

        let args = format!(
            "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
            decoder.width(),
            decoder.height(),
            in_pix_fmt_name,
            ist_time_base.numerator(),
            ist_time_base.denominator(),
            sar_num,
            sar_den,
        );

        graph
            .add(&buffer, "in", &args)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffer filter: {e}"),
            })?;
        graph
            .add(&buffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffersink filter: {e}"),
            })?;

        // Convert pixel format to match encoder's requirement
        let enc_pix_fmt_name = encoder
            .format()
            .descriptor()
            .map_or_else(|| "yuv420p".to_string(), |d| d.name().to_string());

        let format_spec = format!("format=pix_fmts={enc_pix_fmt_name}");

        Self::parse_and_validate_filter_graph(&mut graph, "in", "out", &format_spec)?;

        Ok(graph)
    }

    /// Receive decoded video frames, push through filter, encode, and write.
    pub(super) fn receive_and_process_video(
        decoder: &mut ffmpeg_the_third::decoder::Video,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        filter_time_base: ffmpeg_the_third::Rational,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Video::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("video filter node 'in' not found"))?
                .source()
                .add(&frame)?;
            frame_unref_video(&mut frame);
            Self::drain_video_filter_to_encoder(
                filter,
                encoder,
                octx,
                ost_index,
                enc_time_base,
                filter_time_base,
            )?;
        }
        Ok(())
    }

    /// Pull filtered video frames from filter graph, encode, and write.
    pub(super) fn drain_video_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        filter_time_base: ffmpeg_the_third::Rational,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Video::empty();
        loop {
            let mut out_node = filter.get("out").ok_or_else(|| {
                PostProcessError::ffmpeg_failed("video filter node 'out' not found")
            })?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            // Clear source frame type hints — let the encoder decide GOP structure.
            // Without this, x265 warns "specified frame type is not compatible
            // with max B-frames" on every frame whose source pict_type conflicts.
            filtered.set_kind(ffmpeg_the_third::util::picture::Type::None);

            // Rescale the frame pts from the buffersink/filter tb to the encoder tb
            // (1/fps) so the `cts` fed to the encoder is in frame-tick units. This
            // mirrors the FFmpeg CLI, whose buffersink delivers frames already in
            // the encoder tb. Required for libvvenc/libxavs2, which derive `dts`
            // from a frame-tick model: without this their `dts` and the passed-
            // through `pts` land in different scales → `pts < dts` at the muxer.
            // See the time_base rationale in `video_transcode_phases` Phase 2.
            if let Some(pts) = filtered.pts() {
                filtered.set_pts(Some(Self::rescale_frame_pts(
                    pts,
                    filter_time_base,
                    enc_time_base,
                )?));
            }

            encoder.send_frame(&filtered)?;
            frame_unref_video(&mut filtered);
            Self::drain_video_encoder_packets(encoder, octx, ost_index, enc_time_base)?;
        }
        Ok(())
    }

    /// Receive encoded video packets from encoder and write to output.
    ///
    /// Packets are rescaled from `enc_time_base` to the output stream's
    /// `time_base` (read from octx, which reflects the muxer's final value
    /// set during `write_header`).
    pub(super) fn drain_video_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
    ) -> Result<()> {
        let ost_time_base = octx
            .stream(ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("output stream {ost_index} not found"))
            })?
            .time_base();
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.rescale_ts(enc_time_base, ost_time_base);
            packet.set_stream(ost_index);

            // Capture packet metadata before write
            let pts = packet.pts();
            let dts = packet.dts();
            let size = packet.size() as i32;

            // Direct FFI call to capture raw return code
            // SAFETY: octx and packet are valid; av_interleaved_write_frame takes
            // ownership of the packet buffer and unrefs it on success.
            let ret = unsafe {
                ffmpeg_the_third::ffi::av_interleaved_write_frame(
                    octx.as_mut_ptr(),
                    packet.as_ptr().cast_mut(),
                )
            };
            // Unref immediately: frees encoded data before next receive_packet.
            // av_interleaved_write_frame unrefs on success in FFmpeg 8.0, but NOT
            // on failure. Explicit unref is idempotent and matches
            // merge.rs / remux.rs / thumbnail.rs patterns.
            packet_unref(&mut packet);
            if ret < 0 {
                if ret == -12 {
                    warn!(
                        "FFmpeg allocation failure (ENOMEM) during video mux write — \
                         likely packet/frame leak or extreme memory pressure (rss={}KB)",
                        get_process_rss_kb(),
                    );
                }
                let strerr = av_strerror_string(ret);
                let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                return Err(PostProcessError::MuxWriteError {
                    code: ret,
                    message: format!("ret={ret} ({strerr}), {io_diag}"),
                    operation: "av_interleaved_write_frame (video)".into(),
                    stream_index: ost_index,
                    pts,
                    dts,
                    packet_size: size,
                    time_base_num: ost_time_base.numerator(),
                    time_base_den: ost_time_base.denominator(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::FFmpegRunner;
    use ffmpeg_the_third::Rational;
    use ffmpeg_the_third::format::Pixel;

    // --- enforce_decodable_pixfmt (issue #332) ---

    #[test]
    fn avs2_forces_8bit_yuv420p_from_10bit_source() {
        // libdavs2 can't decode 10-bit AVS2 → a 10-bit pick must be forced to 8-bit.
        assert_eq!(
            FFmpegRunner::enforce_decodable_pixfmt("libxavs2", Pixel::YUV420P10),
            Pixel::YUV420P
        );
    }

    #[test]
    fn avs2_8bit_pick_passes_through_as_yuv420p() {
        assert_eq!(
            FFmpegRunner::enforce_decodable_pixfmt("libxavs2", Pixel::YUV420P),
            Pixel::YUV420P
        );
    }

    #[test]
    fn non_avs2_codecs_keep_their_pixfmt() {
        // libvvenc legitimately encodes 10-bit; must NOT be forced to 8-bit.
        assert_eq!(
            FFmpegRunner::enforce_decodable_pixfmt("libvvenc", Pixel::YUV420P10),
            Pixel::YUV420P10
        );
        // unrelated codec / format is untouched.
        assert_eq!(
            FFmpegRunner::enforce_decodable_pixfmt("libx264", Pixel::NV12),
            Pixel::NV12
        );
    }

    // --- rescale_frame_pts (issue #331) ---

    #[test]
    fn rescale_pts_converts_between_timebases() {
        // 1536 ticks @ 1/12800 = 0.12s; in 1/25 tb that is frame 3.
        assert_eq!(
            FFmpegRunner::rescale_frame_pts(1536, Rational(1, 12800), Rational(1, 25)).unwrap(),
            3
        );
    }

    #[test]
    fn rescale_pts_identity_timebase_is_unchanged() {
        assert_eq!(
            FFmpegRunner::rescale_frame_pts(512, Rational(1, 12800), Rational(1, 12800)).unwrap(),
            512
        );
    }

    #[test]
    fn rescale_pts_errors_on_zero_denominator_source() {
        // Without the guard, av_rescale_q would return INT64_MIN (AV_NOPTS_VALUE)
        // and feed it to the encoder. The guard must surface an error instead.
        assert!(FFmpegRunner::rescale_frame_pts(100, Rational(1, 0), Rational(1, 25)).is_err());
    }

    #[test]
    fn rescale_pts_errors_on_zero_denominator_target() {
        assert!(FFmpegRunner::rescale_frame_pts(100, Rational(1, 25), Rational(1, 0)).is_err());
    }
}
