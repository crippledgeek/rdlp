//! In-process media fixture synthesis for tests (feature `test-support`).
//!
//! rdlp is library-first: nothing in the product spawns an `ffmpeg` binary,
//! and that includes tests. A fixture built by shelling out silently
//! reintroduces the system-binary dependency (self-skips when it is absent,
//! and exercises a different `FFmpeg` build than the one linked). This module
//! generates fixtures through the same `ffmpeg-the-third` bindings and the
//! same encode/mux path the crate itself uses.
//!
//! Lives in `src/` (not `tests/common`) because it needs the crate's
//! `pub(crate)` graph and mux helpers and, like them, `unsafe` FFI — which
//! the crate's tests deny. Gated behind the `test-support` feature so it is
//! never linked into a release binary
//! (`scripts/check-test-only-features-not-in-release.sh`).

use std::path::Path;

use crate::error::{PostProcessError, Result};
use crate::ffmpeg::FFmpegRunner;
use crate::ffmpeg::ffi_helpers::frame_unref_audio;
use crate::ffmpeg::transcode::mux_timing::{MuxTimingState, flush_interleave_queue};

/// A synthetic sine-tone audio fixture.
#[derive(Debug, Clone, Copy)]
pub struct SineAudio {
    /// Tone length in seconds.
    pub duration_secs: f64,
    /// Tone frequency in Hz.
    pub frequency_hz: u32,
    /// Sample rate in Hz (the encoder may pick its nearest supported rate).
    pub sample_rate: u32,
    /// Channel count (a default layout for that count is used).
    pub channels: u8,
    /// Encoder name, or `None` for the output container's default audio codec.
    pub encoder: Option<&'static str>,
}

impl Default for SineAudio {
    fn default() -> Self {
        Self {
            duration_secs: 2.0,
            frequency_hz: 440,
            sample_rate: 48_000,
            channels: 2,
            encoder: None,
        }
    }
}

/// Write a sine-tone audio file at `output`, container chosen by extension.
///
/// Equivalent to `ffmpeg -f lavfi -i sine=… -c:a <encoder> output`, done
/// in-process: a `sine` → `aformat` → `asetnsamples` lavfi graph feeds the
/// crate's own encoder/mux pipeline.
///
/// # Errors
///
/// Any `FFmpeg` failure opening the encoder, building the graph, or writing
/// the container.
pub fn write_sine_audio(output: &Path, spec: &SineAudio) -> Result<()> {
    crate::ffmpeg::ensure_init()?;

    let mut octx = ffmpeg_the_third::format::output(output)?;
    let enc_codec = if let Some(name) = spec.encoder {
        ffmpeg_the_third::encoder::find_by_name(name).ok_or_else(|| {
            PostProcessError::UnsupportedCodec {
                codec: name.to_owned(),
                operation: "fixture synthesis".into(),
            }
        })?
    } else {
        let id = octx
            .format()
            .codec(output, ffmpeg_the_third::media::Type::Audio);
        ffmpeg_the_third::encoder::find(id).ok_or_else(|| {
            PostProcessError::ffmpeg_failed("no default audio encoder for output format")
        })?
    };
    let needs_global_header = octx
        .format()
        .flags()
        .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

    let ost_index = octx.add_stream(enc_codec)?.index();
    // With the codec, so its own defaults apply (see audio_extract.rs, #639).
    let enc_context = ffmpeg_the_third::codec::context::Context::new_with_codec(enc_codec);

    let mut encoder = enc_context.encoder().audio()?;
    let format = FFmpegRunner::pick_audio_sample_format(
        &enc_codec,
        ffmpeg_the_third::format::Sample::F32(ffmpeg_the_third::format::sample::Type::Planar),
    );
    let rate = FFmpegRunner::pick_audio_sample_rate(&enc_codec, spec.sample_rate);
    encoder.set_format(format);
    encoder.set_rate(i32::try_from(rate).map_err(|_| PostProcessError::ffmpeg_failed("rate"))?);
    let enc_time_base = ffmpeg_the_third::Rational(1, i32::try_from(rate).unwrap_or(48_000));
    encoder.set_time_base(enc_time_base);
    // SAFETY: a valid, not-yet-opened encoder context.
    FFmpegRunner::set_default_channel_layout(
        unsafe { encoder.as_mut_ptr() },
        i32::from(spec.channels),
    );
    if needs_global_header {
        // SAFETY: a valid, not-yet-opened encoder context.
        FFmpegRunner::set_global_header_flag(unsafe { encoder.as_mut_ptr() });
    }
    crate::ffmpeg::codec_registry::enable_experimental_if_flagged(&mut encoder, enc_codec);
    let mut encoder = encoder.open_as(enc_codec)?;
    // Re-read after open, as the production path does: avcodec_open2 may
    // replace an encoder's time_base (libavcodec/encode.c fills it only when
    // unset, but that is the encoder's contract to keep, not ours).
    // SAFETY: a valid opened encoder context.
    let enc_time_base = unsafe {
        let tb = (*encoder.as_ptr()).time_base;
        ffmpeg_the_third::Rational(tb.num, tb.den)
    };
    // SAFETY: a valid opened encoder context.
    FFmpegRunner::copy_encoder_params_to_stream(&mut octx, ost_index, unsafe { encoder.as_ptr() });
    octx.write_header()?;

    // Source graph. `asetnsamples` delivers exactly the encoder's frame size
    // (AAC and friends reject anything else with EINVAL); `p=0` lets the last,
    // shorter frame through — libavcodec/encode.c admits exactly one
    // undersized final frame for fixed-frame-size encoders — so the tone ends
    // where `duration` says. A `frame_size()` of 0 means the encoder declares
    // AV_CODEC_CAP_VARIABLE_FRAME_SIZE and accepts any count; 1024 is then
    // just a chunk size.
    let frame_size = if encoder.frame_size() > 0 {
        encoder.frame_size()
    } else {
        1024
    };
    let mut graph = ffmpeg_the_third::filter::Graph::new();
    let sine = ffmpeg_the_third::filter::find("sine")
        .ok_or_else(|| PostProcessError::ffmpeg_failed("sine filter not found"))?;
    let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
        .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;
    graph.add(
        &sine,
        "in",
        &format!(
            "frequency={}:sample_rate={rate}:duration={}",
            spec.frequency_hz, spec.duration_secs
        ),
    )?;
    graph.add(&abuffersink, "out", "")?;
    let layout = match spec.channels {
        1 => "mono".to_owned(),
        2 => "stereo".to_owned(),
        n => format!("{n}c"),
    };
    FFmpegRunner::parse_and_validate_filter_graph(
        &mut graph,
        "in",
        "out",
        &format!(
            "aformat=sample_fmts={}:sample_rates={rate}:channel_layouts={layout},asetnsamples=n={frame_size}:p=0",
            format.name()
        ),
    )?;

    let mut timing = MuxTimingState::default();
    let mut frame = ffmpeg_the_third::frame::Audio::empty();
    loop {
        let mut out = graph
            .get("out")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
        if out.sink().frame(&mut frame).is_err() {
            break;
        }
        encoder.send_frame(&frame)?;
        frame_unref_audio(&mut frame);
        FFmpegRunner::drain_encoder_packets(
            &mut encoder,
            &mut octx,
            ost_index,
            enc_time_base,
            &mut timing,
        )?;
    }
    encoder.send_eof()?;
    FFmpegRunner::drain_encoder_packets(
        &mut encoder,
        &mut octx,
        ost_index,
        enc_time_base,
        &mut timing,
    )?;
    flush_interleave_queue(&mut octx);
    octx.write_trailer()?;
    Ok(())
}

/// A solid-colour still image, `width`×`height`, RGB.
#[derive(Debug, Clone, Copy)]
pub struct StillImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Fill colour as `(r, g, b)`.
    pub rgb: (u8, u8, u8),
}

impl Default for StillImage {
    fn default() -> Self {
        Self {
            width: 64,
            height: 64,
            rgb: (200, 40, 40),
        }
    }
}

/// Write `spec` as a PNG file at `output`.
///
/// One frame through the `png` encoder — a PNG packet *is* a complete PNG
/// file, so no muxer is involved. Equivalent to
/// `ffmpeg -f lavfi -i color=… -frames:v 1 output.png`, done in-process.
///
/// # Errors
///
/// Any `FFmpeg` failure opening the encoder or encoding the frame, or an
/// I/O failure writing the file.
pub fn write_still_png(output: &Path, spec: &StillImage) -> Result<()> {
    crate::ffmpeg::ensure_init()?;

    let codec = ffmpeg_the_third::encoder::find_by_name("png").ok_or_else(|| {
        PostProcessError::UnsupportedCodec {
            codec: "png".into(),
            operation: "fixture synthesis".into(),
        }
    })?;
    let mut encoder = ffmpeg_the_third::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()?;
    encoder.set_width(spec.width);
    encoder.set_height(spec.height);
    encoder.set_format(ffmpeg_the_third::format::Pixel::RGB24);
    encoder.set_time_base(ffmpeg_the_third::Rational(1, 25));
    crate::ffmpeg::codec_registry::enable_experimental_if_flagged(&mut encoder, codec);
    let mut encoder = encoder.open_as(codec)?;

    let mut frame = ffmpeg_the_third::frame::Video::new(
        ffmpeg_the_third::format::Pixel::RGB24,
        spec.width,
        spec.height,
    );
    let stride = frame.stride(0);
    let rgb: [u8; 3] = spec.rgb.into();
    let row_bytes = spec.width as usize * 3;
    for row in frame.data_mut(0).chunks_mut(stride) {
        // Rows are padded to `stride`; only the first `row_bytes` are pixels.
        for px in row
            .iter_mut()
            .take(row_bytes)
            .collect::<Vec<_>>()
            .chunks_mut(3)
        {
            for (dst, src) in px.iter_mut().zip(rgb) {
                **dst = src;
            }
        }
    }
    frame.set_pts(Some(0));

    encoder.send_frame(&frame)?;
    encoder.send_eof()?;
    let mut packet = ffmpeg_the_third::Packet::empty();
    encoder.receive_packet(&mut packet)?;
    let bytes = packet
        .data()
        .ok_or_else(|| PostProcessError::ffmpeg_failed("png encoder produced an empty packet"))?;
    // Test-support only, and the surrounding callers are synchronous tests.
    #[allow(clippy::disallowed_methods)]
    std::fs::write(output, bytes).map_err(|e| {
        PostProcessError::ffmpeg_failed(format!("writing {}: {e}", output.display()))
    })?;
    Ok(())
}

/// Write a sine-tone audio file at `output` with `cover` embedded as its
/// attached picture.
///
/// Goes through the crate's own embed path — the same PNG → baseline-JPEG
/// normalisation `ThumbnailStage` applies (#519) and then
/// `FFmpegRunner::embed_thumbnail` — so the fixture carries exactly the
/// cover-art stream rdlp itself would write (an `ATTACHED_PIC` video-medium
/// stream for MP4-family / FLAC targets).
///
/// # Errors
///
/// Any failure synthesising the tone, the cover, or embedding it.
pub fn write_sine_audio_with_cover(
    output: &Path,
    audio: &SineAudio,
    cover: &StillImage,
) -> Result<()> {
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| PostProcessError::ffmpeg_failed("fixture output needs an extension"))?;
    let plain = output.with_file_name(format!(
        "plain-{}",
        output.file_name().unwrap_or_default().to_string_lossy()
    ));
    let png = output.with_extension("cover.png");
    let jpg = output.with_extension("cover.jpg");
    write_sine_audio(&plain, audio)?;
    write_still_png(&png, cover)?;
    FFmpegRunner::transcode_image_sync(&png, &jpg).map_err(|e| {
        PostProcessError::ffmpeg_failed(format!("normalising fixture cover: {e:#}"))
    })?;
    FFmpegRunner::embed_thumbnail_sync(&plain, &jpg, output, ext, None, None)
        .map_err(|e| PostProcessError::ffmpeg_failed(format!("embedding fixture cover: {e:#}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sine_fixture_is_a_real_audio_only_file_of_the_requested_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.m4a");
        write_sine_audio(
            &path,
            &SineAudio {
                duration_secs: 1.5,
                ..SineAudio::default()
            },
        )
        .unwrap();

        let info = FFmpegRunner::new().unwrap().probe(&path).await.unwrap();
        assert!(info.has_audio && !info.has_video, "{info:?}");
        assert_eq!(info.sample_rate, Some(48_000));
        let duration = info.duration.expect("duration");
        assert!((duration - 1.5).abs() < 0.1, "duration {duration}");
    }

    #[tokio::test]
    async fn still_png_fixture_probes_as_a_png_image_of_the_requested_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cover.png");
        write_still_png(
            &path,
            &StillImage {
                width: 48,
                height: 32,
                ..StillImage::default()
            },
        )
        .unwrap();

        let info = FFmpegRunner::new().unwrap().probe(&path).await.unwrap();
        assert_eq!(
            info.video_codec
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("png"),
            "{info:?}"
        );
        assert_eq!((info.width, info.height), (Some(48), Some(32)));
    }

    /// #643: an audio file whose only video-medium stream is its cover art is
    /// NOT a video source. `ff_add_attached_pic` forces `codec_type = VIDEO`
    /// on the picture (`libavformat/demux_utils.c`), so the medium alone
    /// cannot tell; the `ATTACHED_PIC` disposition can, and the probe must
    /// classify by it.
    #[tokio::test]
    async fn audio_with_cover_art_probes_as_audio_only_with_an_attached_picture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.m4a");
        write_sine_audio_with_cover(&path, &SineAudio::default(), &StillImage::default()).unwrap();

        let info = FFmpegRunner::new().unwrap().probe(&path).await.unwrap();
        assert!(info.has_audio, "{info:?}");
        assert!(
            !info.has_video,
            "cover art must not count as video for routing: {info:?}"
        );
        assert!(info.video_codec.is_none(), "{info:?}");
        assert!(
            info.streams
                .iter()
                .any(|s| s.codec_type == crate::ffmpeg::probe::StreamKind::AttachedPicture),
            "the cover must still be visible as an attached picture: {info:?}"
        );
    }
}
