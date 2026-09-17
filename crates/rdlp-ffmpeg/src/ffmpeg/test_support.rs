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

    let (ost_index, enc_context) = {
        let ost = octx.add_stream(enc_codec)?;
        (
            ost.index(),
            ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?,
        )
    };

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
    // shorter frame through so the tone ends where `duration` says.
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
}
