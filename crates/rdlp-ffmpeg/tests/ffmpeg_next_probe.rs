//! Test to verify ffmpeg-the-third bindings work for media probing.
//!
//! Generates a synthetic test video using FFmpeg library bindings (lavfi
//! sources), then probes it to extract stream info.  No CLI spawning.

use std::path::Path;

/// Generate a 1-second test video using FFmpeg library bindings.
///
/// Creates a minimal MP4 with:
/// - Video: 320x240, 25fps, ~1 second of solid green frames (mpeg4)
/// - Audio: 44100 Hz stereo, ~1 second of silent frames (AAC)
fn generate_test_video(path: &Path) {
    use ffmpeg_the_third::ffi;

    // --- output format context ---
    let mut octx = ffmpeg_the_third::format::output(path).expect("failed to create output format");

    // --- video stream (mpeg4 — built-in, no external library needed) ---
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

    // Copy encoder params to output stream
    unsafe {
        let ost_ptr = (*octx.as_mut_ptr()).streams.add(v_ost_idx);
        ffi::avcodec_parameters_from_context((**ost_ptr).codecpar, v_enc.as_ptr());
    }

    // --- audio stream (AAC) ---
    let aac = ffmpeg_the_third::encoder::find_by_name("aac").expect("aac encoder not found");
    let a_ost = octx.add_stream(aac).expect("failed to add audio stream");
    let a_ost_idx = a_ost.index();
    let a_enc_ctx = ffmpeg_the_third::codec::context::Context::from_parameters(a_ost.parameters())
        .expect("failed to create audio encoder context");
    let mut a_enc = a_enc_ctx
        .encoder()
        .audio()
        .expect("failed to open audio encoder context");
    a_enc.set_format(ffmpeg_the_third::format::Sample::F32(
        ffmpeg_the_third::format::sample::Type::Planar,
    ));
    a_enc.set_rate(44100);
    a_enc.set_time_base(ffmpeg_the_third::Rational(1, 44100));
    a_enc.set_bit_rate(128_000);

    // Set stereo channel layout
    unsafe {
        let ptr = a_enc.as_mut_ptr();
        let mut layout: ffi::AVChannelLayout = std::mem::zeroed();
        ffi::av_channel_layout_default(&mut layout, 2);
        ffi::av_channel_layout_copy(&mut (*ptr).ch_layout, &layout);
        ffi::av_channel_layout_uninit(&mut layout);
    }

    if needs_global {
        unsafe {
            (*a_enc.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    let mut a_enc = a_enc.open_as(aac).expect("failed to open aac encoder");

    // Copy encoder params to output stream
    unsafe {
        let ost_ptr = (*octx.as_mut_ptr()).streams.add(a_ost_idx);
        ffi::avcodec_parameters_from_context((**ost_ptr).codecpar, a_enc.as_ptr());
    }

    // --- write header ---
    octx.write_header().expect("failed to write output header");

    // --- generate video frames (25 frames = 1 second of green) ---
    let frame_count = 25u32;
    for i in 0..frame_count {
        let mut frame =
            ffmpeg_the_third::frame::Video::new(ffmpeg_the_third::format::Pixel::YUV420P, 320, 240);
        frame.set_pts(Some(i64::from(i)));

        // Fill Y plane with 149 (green in YUV), U=43, V=21
        let y_data = frame.data_mut(0);
        for byte in y_data.iter_mut() {
            *byte = 149;
        }
        let u_data = frame.data_mut(1);
        for byte in u_data.iter_mut() {
            *byte = 43;
        }
        let v_data = frame.data_mut(2);
        for byte in v_data.iter_mut() {
            *byte = 21;
        }

        v_enc
            .send_frame(&frame)
            .expect("failed to send video frame");
        drain_video_packets(&mut v_enc, &mut octx, v_ost_idx);
    }
    v_enc.send_eof().expect("failed to send video eof");
    drain_video_packets(&mut v_enc, &mut octx, v_ost_idx);

    // --- generate audio frames (~1 second of silence) ---
    let audio_frame_size = if a_enc.frame_size() > 0 {
        a_enc.frame_size()
    } else {
        1024
    };
    let total_samples = 44100u32;
    let mut samples_written = 0u64;

    while samples_written < u64::from(total_samples) {
        let mut frame = ffmpeg_the_third::frame::Audio::new(
            ffmpeg_the_third::format::Sample::F32(ffmpeg_the_third::format::sample::Type::Planar),
            audio_frame_size as usize,
            ffmpeg_the_third::util::channel_layout::ChannelLayoutMask::STEREO,
        );
        frame.set_pts(Some(samples_written as i64));
        frame.set_rate(44100);

        // Silence: zero-filled by default
        a_enc
            .send_frame(&frame)
            .expect("failed to send audio frame");
        drain_audio_packets(&mut a_enc, &mut octx, a_ost_idx);
        samples_written += u64::from(audio_frame_size);
    }
    a_enc.send_eof().expect("failed to send audio eof");
    drain_audio_packets(&mut a_enc, &mut octx, a_ost_idx);

    // --- write trailer ---
    octx.write_trailer()
        .expect("failed to write output trailer");
}

fn drain_video_packets(
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

fn drain_audio_packets(
    encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
    octx: &mut ffmpeg_the_third::format::context::Output,
    ost_idx: usize,
) {
    let mut packet = ffmpeg_the_third::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(ost_idx);
        packet.write_interleaved(octx).ok();
    }
}

#[test]
fn test_ffmpeg_probe() {
    ffmpeg_the_third::init().expect("ffmpeg init failed");

    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let video_path = dir.path().join("test.mp4");

    generate_test_video(&video_path);

    let ctx = ffmpeg_the_third::format::input(&video_path).expect("failed to open test video");

    // Check duration (~1 second)
    let duration_secs = ctx.duration() as f64 / f64::from(ffmpeg_the_third::ffi::AV_TIME_BASE);
    assert!(
        duration_secs > 0.5 && duration_secs < 2.0,
        "unexpected duration: {duration_secs}"
    );

    // Should have at least 2 streams (video + audio)
    let stream_count = ctx.streams().count();
    assert!(
        stream_count >= 2,
        "expected >= 2 streams, got {stream_count}"
    );

    // Find video stream
    let video_stream = ctx
        .streams()
        .best(ffmpeg_the_third::media::Type::Video)
        .expect("no video stream found");

    let video_codec =
        ffmpeg_the_third::codec::context::Context::from_parameters(video_stream.parameters())
            .expect("failed to get video codec context");

    assert_eq!(video_codec.medium(), ffmpeg_the_third::media::Type::Video);

    if let Ok(video) = video_codec.decoder().video() {
        assert_eq!(video.width(), 320);
        assert_eq!(video.height(), 240);
        println!(
            "Video: {}x{}, format: {:?}",
            video.width(),
            video.height(),
            video.format()
        );
    }

    // Find audio stream
    let audio_stream = ctx
        .streams()
        .best(ffmpeg_the_third::media::Type::Audio)
        .expect("no audio stream found");

    let audio_codec =
        ffmpeg_the_third::codec::context::Context::from_parameters(audio_stream.parameters())
            .expect("failed to get audio codec context");

    assert_eq!(audio_codec.medium(), ffmpeg_the_third::media::Type::Audio);

    if let Ok(audio) = audio_codec.decoder().audio() {
        assert!(audio.rate() > 0, "audio sample rate should be > 0");
        println!(
            "Audio: {} Hz, channel_layout: {:?}, format: {:?}",
            audio.rate(),
            audio.ch_layout(),
            audio.format()
        );
    }

    println!("ffmpeg probe test passed (libav-generated test video)");
}
