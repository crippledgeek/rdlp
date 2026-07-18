//! Still-image normalization to baseline MJPEG (`.jpg`).
//!
//! `transcode_image` decodes a single still image in any `FFmpeg`-readable
//! format (webp, png, …) and re-encodes it as baseline MJPEG in a `.jpg`
//! container. This is the single normalization point for "make this image
//! MP4-embeddable" — MP4/MOV/M4A/M4V have no muxer tag for `webp` (or most
//! other still-image codecs), so `ThumbnailStage` calls this before both the
//! `embed_thumbnail` `ATTACHED_PIC` pass and the `mp4ameta` `covr` atom pass
//! for any container that isn't Matroska (which attaches the source codec
//! natively; see `thumbnail/mkv_raw_ffi.rs`).
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` APIs use mixed C integer types; all casts are
//!   audited and within valid ranges for `FFmpeg`-returned values.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]

use std::path::Path;

use anyhow::Context as _;

use crate::error::{PostProcessError, Result};

use super::super::{FFmpegRunner, ensure_init};

/// MJPEG `qscale` value used when normalizing a thumbnail image, set via
/// `AV_CODEC_FLAG_QSCALE` and `global_quality` (`FFmpegRunner::set_vbr_quality`).
/// `FFmpeg`'s `mjpeg` encoder quantizer scale runs 1 (best) to 31 (worst); 3
/// matches the `FFmpeg` CLI's default `-q:v` range for "visually lossless"
/// JPEG output and keeps a thumbnail-sized image small without visible blocking.
const THUMBNAIL_MJPEG_QSCALE: i32 = 3;

/// Maximum accepted thumbnail edge length, in pixels (decompression-bomb guard).
///
/// Thumbnails are attacker-controlled (served by the video site) and the
/// download layer caps only the *encoded* file size (`MAX_THUMBNAIL_BYTES`),
/// not decoded dimensions — a small, size-cap-compliant still image can declare
/// an enormous canvas that decodes to a multi-gigabyte raw frame. 8192 is
/// generous for any legitimate cover image (4K is 3840 wide) while bounding the
/// decode buffer to a safe size. Oversized inputs are rejected before any frame
/// buffer is allocated; `ThumbnailStage` then falls back to the original file
/// (the embed is non-fatal).
const MAX_THUMBNAIL_DIMENSION: u32 = 8192;

/// Reject a thumbnail whose declared canvas exceeds [`MAX_THUMBNAIL_DIMENSION`]
/// on either axis, before any decode buffer is allocated.
fn ensure_thumbnail_dimensions(width: u32, height: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        width <= MAX_THUMBNAIL_DIMENSION && height <= MAX_THUMBNAIL_DIMENSION,
        "thumbnail dimensions {width}x{height} exceed the {MAX_THUMBNAIL_DIMENSION}px cap"
    );
    Ok(())
}

impl FFmpegRunner {
    /// Normalize a still image to baseline MJPEG (`.jpg`) on a background thread.
    ///
    /// # Errors
    ///
    /// Returns an error if `FFmpeg` fails to open the input, find a video
    /// stream, decode the frame, open the `mjpeg` encoder, or write/mux the
    /// output.
    pub async fn transcode_image(
        &self,
        src: impl AsRef<Path>,
        dst: impl AsRef<Path>,
    ) -> Result<()> {
        let src = src.as_ref().to_path_buf();
        let dst = dst.as_ref().to_path_buf();
        Self::spawn_blocking("transcode_image", move || -> Result<()> {
            Ok(Self::transcode_image_sync(&src, &dst)?)
        })
        .await
    }

    /// Decode the first video frame of `src` and encode it as baseline MJPEG to `dst`.
    fn transcode_image_sync(src: &Path, dst: &Path) -> anyhow::Result<()> {
        ensure_init()?;

        // FFmpeg's still-image decoders are chatty at info/warning level about
        // things like unsupported ICC profiles; we only care about hard failures.
        let _suppress = super::super::log_capture::LogSuppressGuard::error_level();

        let mut ictx = ffmpeg_the_third::format::input(src)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to open image input {}", src.display()))?;

        let ist = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .ok_or(PostProcessError::NoVideoStream)?;
        let ist_index = ist.index();
        let ist_time_base = ist.time_base();

        let dec_ctx = ffmpeg_the_third::codec::context::Context::from_parameters(ist.parameters())?;
        let mut decoder = dec_ctx.decoder().video()?;

        // Decompression-bomb guard: reject an oversized declared canvas before
        // any frame buffer is allocated (the thumbnail is attacker-controlled).
        // `decoder.width()/height()` come from the container header (parsed by
        // `find_stream_info`), so this fires before the codec sizes its per-pixel
        // decode buffer — the load-bearing property. Still-image codecs
        // (webp/png/jpeg) don't renegotiate resolution mid-stream, so the header
        // dims are authoritative here.
        ensure_thumbnail_dimensions(decoder.width(), decoder.height())
            .with_context(|| format!("thumbnail {} rejected", src.display()))?;

        let mut octx = ffmpeg_the_third::format::output(dst)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to create image output {}", dst.display()))?;

        let mjpeg_codec = ffmpeg_the_third::encoder::find_by_name("mjpeg").ok_or_else(|| {
            PostProcessError::UnsupportedCodec {
                codec: "mjpeg".to_string(),
                operation: "thumbnail image normalization".into(),
            }
        })?;

        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        let ost_index;
        {
            let ost = octx
                .add_stream(mjpeg_codec)
                .map_err(PostProcessError::from)
                .context("failed to add output stream for image normalization")?;
            ost_index = ost.index();
        }

        let enc_context = ffmpeg_the_third::codec::context::Context::new_with_codec(mjpeg_codec);
        let mut encoder = enc_context.encoder().video()?;
        encoder.set_width(decoder.width());
        encoder.set_height(decoder.height());
        let target_pix_fmt = Self::pick_video_pixel_format(&mjpeg_codec, decoder.format());
        encoder.set_format(target_pix_fmt);

        // Still-image decoders (webp, png, …) commonly produce full-range
        // (JPEG-range) YUV frames on a plain (non-`YUVJ*`) pixel format —
        // `pick_video_pixel_format` picks the encoder-supported format closest
        // to the decoder's, which for `mjpeg` is typically the same plain
        // format. At the default `Normal` compliance level libavcodec's mjpeg
        // encoder rejects that combination ("Non full-range YUV is
        // non-standard"). `Unofficial` is exactly the compliance level
        // FFmpeg's own error message names as the fix, and is safe here: this
        // encoder is used only for thumbnail normalization, never a
        // spec-sensitive delivery path.
        encoder.compliance(ffmpeg_the_third::codec::Compliance::Unofficial);

        // A still image is a single-frame stream; the tick rate is otherwise
        // meaningless. 1/1 (one frame per "second") is the conventional choice
        // FFmpeg's own image2 muxer path uses for single-frame output.
        let enc_time_base = ffmpeg_the_third::Rational(1, 1);
        encoder.set_time_base(enc_time_base);
        encoder.set_frame_rate(Some(enc_time_base));

        if needs_global_header {
            // SAFETY: encoder is a valid pre-open encoder context.
            Self::set_global_header_flag(unsafe { encoder.as_mut_ptr() });
        }
        // SAFETY: encoder is a valid pre-open encoder context.
        Self::set_vbr_quality(unsafe { encoder.as_mut_ptr() }, THUMBNAIL_MJPEG_QSCALE);

        let mut encoder = encoder
            .open_as(mjpeg_codec)
            .map_err(PostProcessError::from)
            .context("failed to open mjpeg encoder for image normalization")?;

        // SAFETY: octx owns ost_index (just added above); encoder was just opened.
        Self::copy_encoder_params_to_stream(&mut octx, ost_index, unsafe { encoder.as_ptr() });

        octx.write_header()
            .map_err(PostProcessError::from)
            .context("failed to write output header for image normalization")?;

        let mut filter_graph = Self::build_video_filter(&decoder, &encoder, ist_time_base)?;

        for result in ictx.packets() {
            let (stream, packet) = result
                .map_err(PostProcessError::from)
                .context("failed to read packet during image normalization")?;
            if stream.index() != ist_index {
                continue;
            }
            decoder.send_packet(&packet)?;
            Self::receive_and_process_video(
                &mut decoder,
                &mut filter_graph,
                &mut encoder,
                &mut octx,
                ost_index,
                enc_time_base,
                ist_time_base,
            )?;
        }

        // Flush decoder -> filter -> encoder.
        decoder.send_eof()?;
        Self::receive_and_process_video(
            &mut decoder,
            &mut filter_graph,
            &mut encoder,
            &mut octx,
            ost_index,
            enc_time_base,
            ist_time_base,
        )?;
        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("image filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_video_filter_to_encoder(
            &mut filter_graph,
            &mut encoder,
            &mut octx,
            ost_index,
            enc_time_base,
            ist_time_base,
        )?;
        encoder.send_eof()?;
        Self::drain_video_encoder_packets(&mut encoder, &mut octx, ost_index, enc_time_base)?;

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for image normalization")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_THUMBNAIL_DIMENSION, ensure_thumbnail_dimensions};

    #[test]
    fn dimensions_at_or_below_cap_accepted() {
        assert!(ensure_thumbnail_dimensions(1, 1).is_ok());
        // The cap itself (both axes) is accepted — pins the boundary on the pass side.
        assert!(
            ensure_thumbnail_dimensions(MAX_THUMBNAIL_DIMENSION, MAX_THUMBNAIL_DIMENSION).is_ok()
        );
    }

    #[test]
    fn dimensions_one_over_cap_rejected() {
        // cap + 1 on either axis is rejected — a `>=`/`>` off-by-one flips exactly one.
        assert!(ensure_thumbnail_dimensions(MAX_THUMBNAIL_DIMENSION + 1, 1).is_err());
        assert!(ensure_thumbnail_dimensions(1, MAX_THUMBNAIL_DIMENSION + 1).is_err());
        assert!(
            ensure_thumbnail_dimensions(MAX_THUMBNAIL_DIMENSION + 1, MAX_THUMBNAIL_DIMENSION + 1)
                .is_err()
        );
    }
}
