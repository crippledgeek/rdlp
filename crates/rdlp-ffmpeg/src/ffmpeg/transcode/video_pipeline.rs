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
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Video::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("video filter node 'in' not found"))?
                .source()
                .add(&frame)?;
            frame_unref_video(&mut frame);
            Self::drain_video_filter_to_encoder(filter, encoder, octx, ost_index, enc_time_base)?;
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
