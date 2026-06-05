//! End-to-end: rdlp can recode to the newly-wired VVC/EVC/AVS2 encoders and the
//! result muxes into the right container with the expected codec. Exercises the
//! real `FFmpegRunner::convert_video` transcode path (filtergraph + encoder open
//! + mux), including libvvenc's 10-bit pixel-format requirement.
//!
//! Self-skips when the system `ffmpeg`/`ffprobe` CLI is absent (used only to
//! build the fixture / read back the codec) or when a given encoder is not
//! compiled into the linked FFmpeg.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use rdlp_ffmpeg::ffmpeg::video_codecs::is_encoder_available;
use rdlp_ffmpeg::{FFmpegRunner, VideoConvertOptions};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Command::new("ffprobe")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Build a 1s H.264 yuv420p fixture (a typical post-download source).
fn build_h264_fixture(dir: &Path) -> Result<std::path::PathBuf, ()> {
    let src = dir.join("src.mp4");
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=1:s=320x240",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            src.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(src) } else { Err(()) }
}

/// ffprobe the first video stream's codec_name.
fn probe_video_codec(path: &Path) -> Option<String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            path.to_str().unwrap(),
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[tokio::test]
async fn new_codecs_recode_and_mux() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg/ffprobe not available");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Ok(src) = build_h264_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };

    // Creating the runner initializes the FFmpeg libs (so is_encoder_available works).
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    // EVC (libxeve) and VVC (libvvenc) recode cleanly end-to-end into their
    // containers — assert them. VVC was fixed by aligning the encoder time_base
    // with the frames fed to it and rescaling each frame's pts to the encoder
    // (1/fps) tick base before `send_frame`: libvvenc derives `dts` from a
    // frame-tick model while echoing `cts` (= frame->pts) verbatim, so feeding
    // buffersink-tb pts produced `pts < dts` at the muxer. See the time_base
    // rationale in `video_transcode_phases` Phase 2 / `drain_video_filter_to_encoder`.
    //
    // AVS2 (libxavs2) still FAILS with non-monotonic DTS — a SEPARATE, independent
    // defect: the AVS2 codec descriptor lacks AV_CODEC_PROP_REORDER, so FFmpeg's
    // encode path forces `dts = pts` (encode.c) even though libxavs2 emits a
    // B-frame-reordered stream (bf=7 default). Packets then carry dts==pts in
    // coding order → non-monotonic DTS at the muxer. Not fixable from rdlp's side
    // without an FFmpeg patch adding the prop; tracked as a known-issue.
    let assert_cases = [
        ("libxeve", "mkv", "evc"),
        ("libxeve", "mp4", "evc"),
        ("libvvenc", "mkv", "vvc"),
        ("libvvenc", "mp4", "vvc"),
    ];
    let log_cases = [("libxavs2", "mkv")];

    let mut asserted = 0;
    for (enc, cont, expected) in assert_cases {
        if !is_encoder_available(enc) {
            eprintln!("[SKIP] {enc} not in this FFmpeg build");
            continue;
        }
        let out = dir.path().join(format!("out_{enc}.{cont}"));
        let opts = VideoConvertOptions {
            remux_only: false,
            video_codec: Some(enc.to_string()),
            audio_copy: false,
            ..Default::default()
        };
        let res = runner.convert_video(&src, &out, &opts, None, None).await;
        assert!(res.is_ok(), "{enc} -> {cont} recode failed: {res:?}");
        assert_eq!(
            probe_video_codec(&out).as_deref(),
            Some(expected),
            "{enc} -> {cont}: expected codec {expected}"
        );
        eprintln!("OK   {enc} -> {cont}  codec={expected}");
        asserted += 1;
    }

    // Informational: document the known AVS2 non-monotonic-DTS failure
    // (missing AV_CODEC_PROP_REORDER → FFmpeg forces dts=pts on a reordered stream).
    for (enc, cont) in log_cases {
        if !is_encoder_available(enc) {
            continue;
        }
        let out = dir.path().join(format!("known_{enc}.{cont}"));
        let opts = VideoConvertOptions {
            remux_only: false,
            video_codec: Some(enc.to_string()),
            audio_copy: false,
            ..Default::default()
        };
        match runner.convert_video(&src, &out, &opts, None, None).await {
            Ok(()) => eprintln!(
                "NOTE {enc} -> {cont}: now succeeds — the transcode-timestamp follow-up may be fixed; promote to a hard assert."
            ),
            Err(e) => eprintln!("KNOWN-ISSUE {enc} -> {cont}: {e:?}"),
        }
    }

    if asserted == 0 {
        eprintln!("[SKIP] no asserted encoder (libxeve/libvvenc) built into this FFmpeg");
    }
}
