//! Fixture-backed coverage for `FFmpegRunner::probe` metadata extraction.
//!
//! `probe_sync`'s format- and stream-level metadata parsing has no in-crate
//! unit coverage (its only unit test exercises `StreamInfo::default`). This
//! integration test generates a minimal MKV *with* format- and stream-level
//! metadata tags using FFmpeg library bindings (no CLI spawning), then probes
//! it through the public async API and asserts the tags land in the returned
//! `MediaInfo`. It guards the `.map().collect()` metadata rewrite (issue #475):
//! a regression that dropped the maps would surface here as empty metadata.

// This test directly accesses raw FFmpeg FFI to copy encoder params onto the
// output stream — there is no safe abstraction for it in ffmpeg-the-third v4.1.
#![allow(unsafe_code)]
// expect()/unwrap() are intentional in tests — panics surface failures.
// Integration tests aren't seen as #[cfg(test)] by clippy, so the file-scope
// allow is required (see clippy.toml, rust-clippy#13981).
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::path::Path;

use rdlp_ffmpeg::{FFmpegRunner, StreamKind};

/// Generate a minimal 5-frame MKV (mpeg4 video) carrying format- and
/// stream-level `title` metadata tags.
fn generate_fixture_with_metadata(path: &Path) {
    use ffmpeg_the_third::ffi;

    let mut octx = ffmpeg_the_third::format::output(path).expect("failed to create output format");

    // mpeg4 is a built-in encoder, needs no external library.
    let mpeg4 = ffmpeg_the_third::encoder::find_by_name("mpeg4").expect("mpeg4 encoder not found");
    let v_ost = octx.add_stream(mpeg4).expect("failed to add video stream");
    let v_ost_idx = v_ost.index();
    let v_enc_ctx = ffmpeg_the_third::codec::context::Context::from_parameters(v_ost.parameters())
        .expect("failed to create video encoder context");
    let mut v_enc = v_enc_ctx
        .encoder()
        .video()
        .expect("failed to open video encoder context");
    v_enc.set_width(320);
    v_enc.set_height(240);
    v_enc.set_format(ffmpeg_the_third::format::Pixel::YUV420P);
    v_enc.set_time_base(ffmpeg_the_third::Rational(1, 25));
    v_enc.set_bit_rate(400_000);
    unsafe {
        let ptr = v_enc.as_mut_ptr();
        (*ptr).gop_size = 12;
        (*ptr).max_b_frames = 0;
    }

    let needs_global = octx
        .format()
        .flags()
        .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);
    if needs_global {
        unsafe {
            (*v_enc.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    let mut v_enc = v_enc.open_as(mpeg4).expect("failed to open mpeg4 encoder");

    // Copy encoder params onto the output stream.
    unsafe {
        let ost_ptr = (*octx.as_mut_ptr()).streams.add(v_ost_idx);
        ffi::avcodec_parameters_from_context((**ost_ptr).codecpar, v_enc.as_ptr());
    }

    // Format-level metadata (Matroska SegmentInfo Title).
    let mut fmeta = ffmpeg_the_third::Dictionary::new();
    fmeta.set("title", "Rdlp Probe Fixture");
    octx.set_metadata(fmeta);

    // Stream-level metadata (Matroska track Name).
    let mut smeta = ffmpeg_the_third::Dictionary::new();
    smeta.set("title", "green-test-track");
    octx.stream_mut(v_ost_idx)
        .expect("output stream exists")
        .set_metadata(smeta);

    octx.write_header().expect("failed to write output header");

    // 5 frames of solid green is enough for a valid, probeable container.
    for i in 0..5u32 {
        let mut frame =
            ffmpeg_the_third::frame::Video::new(ffmpeg_the_third::format::Pixel::YUV420P, 320, 240);
        frame.set_pts(Some(i64::from(i)));
        for byte in frame.data_mut(0).iter_mut() {
            *byte = 149; // Y
        }
        for byte in frame.data_mut(1).iter_mut() {
            *byte = 43; // U
        }
        for byte in frame.data_mut(2).iter_mut() {
            *byte = 21; // V
        }
        v_enc.send_frame(&frame).expect("failed to send frame");
        drain(&mut v_enc, &mut octx, v_ost_idx);
    }
    v_enc.send_eof().expect("failed to send eof");
    drain(&mut v_enc, &mut octx, v_ost_idx);

    octx.write_trailer()
        .expect("failed to write output trailer");
}

fn drain(
    encoder: &mut ffmpeg_the_third::encoder::video::Video,
    octx: &mut ffmpeg_the_third::format::context::Output,
    ost_idx: usize,
) {
    let mut packet = ffmpeg_the_third::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(ost_idx);
        packet.write_interleaved(octx).ok();
    }
}

#[tokio::test]
async fn probe_extracts_format_and_stream_metadata() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let video_path = dir.path().join("meta.mkv");
    generate_fixture_with_metadata(&video_path);

    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");
    let info = runner.probe(&video_path).await.expect("probe failed");

    // Format-level metadata round-trips into the map (guards the collect
    // rewrite — a broken collect would leave this empty).
    assert!(
        !info.metadata.is_empty(),
        "expected format metadata, got none"
    );
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Rdlp Probe Fixture"),
        "format-level title tag missing or wrong: {:?}",
        info.metadata,
    );
    assert!(
        info.metadata.keys().all(|k| k == &k.to_lowercase()),
        "format metadata keys must be lowercased: {:?}",
        info.metadata.keys().collect::<Vec<_>>(),
    );

    // Stream-level metadata round-trips into the per-stream map.
    let video = info
        .streams
        .iter()
        .find(|s| s.codec_type == StreamKind::Video)
        .expect("expected a video stream");
    assert_eq!(
        video.metadata.get("title").map(String::as_str),
        Some("green-test-track"),
        "stream-level title tag missing or wrong: {:?}",
        video.metadata,
    );
    assert!(
        video.metadata.keys().all(|k| k == &k.to_lowercase()),
        "stream metadata keys must be lowercased: {:?}",
        video.metadata.keys().collect::<Vec<_>>(),
    );
}
