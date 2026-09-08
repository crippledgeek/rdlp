//! Regression suite for #577: a remux into an audio-only container must not
//! silently carry the input's video stream through.
//!
//! rdlp's remux is a stream copy, and FFmpeg enforces nothing here — the ASF
//! muxer is registered once for `asf,wmv,wma` with no extension-conditioned
//! codec checks, so `-c copy` into a `.wma` writes H.264 and exits 0
//! (reproduced with the system CLI while writing this suite). The extension
//! then says audio-only and the bytes disagree.
//!
//! The decision (#577) is to **refuse**, not to drop the video stream:
//! discarding a user's video track silently is worse than naming the wrong
//! container, and it matches upstream — yt-dlp's `--remux-video` is a stream
//! copy that hard-fails rather than falling back to a re-encode.
//!
//! The carve-out these tests exist to pin: cover art is carried as a
//! *video-codec* stream with the `ATTACHED_PIC` disposition, and every audio
//! container rdlp embeds a thumbnail into has one. A guard written as "input
//! has a video stream → refuse" would break thumbnail embedding for every
//! audio container, so `flac_with_cover_art_still_remuxes` /
//! `mp3_with_cover_art_still_remuxes` below are the load-bearing tests, not
//! the refusal itself.
//!
//! Requires the real `ffmpeg`/`ffprobe` CLI on `PATH` (used only to build
//! fixtures and to verify results independently of rdlp's own decode path).
//! Fails closed: panics unless `RDLP_ALLOW_SKIP_FFMPEG_TESTS` is set, so a
//! machine missing the CLI cannot report a green, assertion-free suite.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions, video_alternative_for};

mod common;
use common::decoded_audio_frames;
use rdlp_types::ContainerFormat;
use strum::IntoEnumIterator as _;

/// Size of the refusal set, asserted after the matrix loop so a future edit
/// that empties or silently shrinks it fails instead of passing vacuously.
/// The twelve are pinned per-container in `audio_only_container.rs`'s unit
/// tests; this is the integration suite's guard against a vacuous loop.
const EXPECTED_REFUSED_CONTAINERS: usize = 12;

fn ffmpeg_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Require `ffmpeg`/`ffprobe`. Returns `true` when the caller should proceed;
/// panics unless `RDLP_ALLOW_SKIP_FFMPEG_TESTS` is explicitly set, mirroring
/// `remux_avi_codec_tag.rs`'s fail-closed skip policy.
fn require_ffmpeg() -> bool {
    if ffmpeg_available() {
        return true;
    }
    if std::env::var_os("RDLP_ALLOW_SKIP_FFMPEG_TESTS").is_some() {
        eprintln!("[SKIP] ffmpeg/ffprobe not available (RDLP_ALLOW_SKIP_FFMPEG_TESTS set)");
        return false;
    }
    panic!(
        "ffmpeg/ffprobe not found on PATH. This suite needs the real CLI to build \
         fixtures and independently verify output. Set RDLP_ALLOW_SKIP_FFMPEG_TESTS=1 \
         to explicitly opt into skipping it."
    );
}

/// Fixtures are built once per process into `CARGO_TARGET_TMPDIR` rather than
/// a `TempDir`: a `static` is never dropped, so a `TempDir` here could never
/// run its cleanup, and this machine's `/tmp` is a RAM-backed tmpfs.
fn fixture_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("audio_only_container_rejects_video");
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("create fixture dir {}: {e}", dir.display()));
    dir
}

struct Fixtures {
    dir: PathBuf,
    /// h264 video + aac audio. The video-bearing source every refusal case uses.
    video_mp4: PathBuf,
    /// flac audio + an `ATTACHED_PIC` mjpeg cover.
    cover_flac: PathBuf,
    /// mp3 audio + an `ATTACHED_PIC` mjpeg cover (an ID3v2 APIC frame).
    cover_mp3: PathBuf,
    /// aac audio and nothing else — no video stream of any kind.
    audio_only_m4a: PathBuf,
}

static FIXTURES: OnceLock<Fixtures> = OnceLock::new();

fn run_ffmpeg(label: &str, args: &[&str], out: &Path) -> PathBuf {
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(args)
        .arg(out)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn ffmpeg for the {label} fixture: {e}"));
    assert!(status.success(), "{label} fixture build failed");
    out.to_path_buf()
}

fn fixtures() -> &'static Fixtures {
    FIXTURES.get_or_init(|| {
        let dir = fixture_dir();

        let video_mp4 = run_ffmpeg(
            "video_mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc=d=1:s=320x240:r=25",
                "-f",
                "lavfi",
                "-i",
                "sine=d=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "64k",
                "-shortest",
            ],
            &dir.join("video.mp4"),
        );

        let cover_jpg = run_ffmpeg(
            "cover_jpg",
            &["-f", "lavfi", "-i", "color=c=red:s=64x64", "-frames:v", "1"],
            &dir.join("cover.jpg"),
        );
        let cover = cover_jpg.to_string_lossy().into_owned();

        let cover_flac = run_ffmpeg(
            "cover_flac",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=d=1",
                "-i",
                &cover,
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "flac",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
            ],
            &dir.join("cover.flac"),
        );

        let cover_mp3 = run_ffmpeg(
            "cover_mp3",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=d=1",
                "-i",
                &cover,
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "libmp3lame",
                "-c:v",
                "copy",
                "-id3v2_version",
                "3",
                "-disposition:v",
                "attached_pic",
            ],
            &dir.join("cover.mp3"),
        );

        let audio_only_m4a = run_ffmpeg(
            "audio_only_m4a",
            &[
                "-f", "lavfi", "-i", "sine=d=1", "-c:a", "aac", "-b:a", "64k",
            ],
            &dir.join("audio_only.m4a"),
        );

        Fixtures {
            dir,
            video_mp4,
            cover_flac,
            cover_mp3,
            audio_only_m4a,
        }
    })
}

/// Whether `path` carries a video-medium stream that is NOT an attached
/// picture — i.e. the same question the guard has to answer, asked through the
/// system CLI instead of through rdlp's own code.
fn has_real_video(path: &Path) -> bool {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "stream_disposition=attached_pic",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn ffprobe on {}: {e}", path.display()));
    assert!(out.status.success(), "ffprobe failed on {}", path.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.trim() == "0")
}

/// The headline case from the issue: `--remux=wma` against a video-bearing
/// source must be refused, and the message must name both the container the
/// user asked for and the one they should have asked for.
///
/// Fails against unpatched code, which stream-copies h264 into the `.wma` and
/// returns `Ok(())` (verified independently: `ffmpeg -i src.mp4 -c copy
/// out.wma` exits 0 and `ffprobe` reports `codec_name=h264`).
#[tokio::test]
async fn wma_target_refuses_a_real_video_stream() {
    if !require_ffmpeg() {
        return;
    }
    let dst = fixtures().dir.join("video_to.wma");
    let _ = std::fs::remove_file(&dst);
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let err = runner
        .remux(&fixtures().video_mp4, &dst, &RemuxOptions::default(), None)
        .await
        .expect_err("wma is audio-only; a video-bearing remux into it must be refused");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("wma"),
        "error must name the container the user asked for, got: {msg}"
    );
    assert!(
        msg.contains("wmv"),
        "error must point at the container video belongs in, got: {msg}"
    );
    assert!(
        !dst.exists(),
        "a refusal must not leave a partial output file at {}",
        dst.display()
    );
}

/// Positive control: the same source into a video-capable container still
/// remuxes, and the video survives. Pins the guard to audio-only targets
/// rather than to "any remux of a video-bearing input".
///
/// `.mov`, deliberately, NOT `.mkv`: an `.mkv` target returns via
/// `remux_mkv_raw_ffi` *before* the guard runs, so an mkv control would pass
/// even if `video_alternative_for` wrongly returned `Some` for every
/// container. `.mov` takes the generic path and actually executes the guard's
/// `None` branch.
#[tokio::test]
async fn video_capable_target_still_remuxes_video() {
    if !require_ffmpeg() {
        return;
    }
    let dst = fixtures().dir.join("video_to.mov");
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    runner
        .remux(&fixtures().video_mp4, &dst, &RemuxOptions::default(), None)
        .await
        .unwrap_or_else(|e| panic!("mov is video-capable; this remux must succeed: {e:#}"));

    assert!(
        has_real_video(&dst),
        "the video stream must survive a remux into a video-capable container"
    );
}

/// The guard's commonest path, and the one nothing else covers: an input with
/// **no video stream at all** into an audio-only target is ordinary work and
/// must succeed. `--remux=wma` on an audio-only download is exactly the case
/// `.wma` exists for.
///
/// Without this, a guard mutated to refuse whenever the target is audio-only —
/// ignoring the input entirely — is caught only by the two cover-art tests,
/// i.e. by the carve-out rather than by the rule.
#[tokio::test]
async fn an_audio_only_source_still_remuxes_into_an_audio_only_target() {
    if !require_ffmpeg() {
        return;
    }
    assert!(
        !has_real_video(&fixtures().audio_only_m4a),
        "fixture precondition: this source must carry no video stream at all"
    );

    let dst = fixtures().dir.join("audio_to.wma");
    let _ = std::fs::remove_file(&dst);
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    runner
        .remux(
            &fixtures().audio_only_m4a,
            &dst,
            &RemuxOptions::default(),
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("an audio-only source into .wma must succeed: {e:#}"));

    // Decoded frames, not `len() > 0`: a failed mux still leaves a non-empty
    // header on disk, so a size check scores a failure as a success. An exact
    // count equality with the source would be wrong here — measured, the
    // `.wma` decodes 45 frames against the `.m4a`'s 44, because AAC priming
    // samples do not survive the container change.
    assert!(
        decoded_audio_frames(&dst).is_some_and(|frames| frames > 0),
        "the remuxed audio must actually decode, not merely exist"
    );
    assert!(
        !has_real_video(&dst),
        "no video stream should have appeared from nowhere"
    );
}

/// The carve-out, case 1: a `.flac` carrying an `ATTACHED_PIC` cover has a
/// video-codec stream, and `Flac` is one of the containers the guard refuses
/// video for. It must still remux — the thumbnail-embed path
/// (`ThumbnailEmbedStrategy::FlacAttachedPic`) puts that stream there
/// deliberately.
///
/// Would fail against a guard written as "input has a video stream → refuse".
///
/// `.m4a` was the first choice here and does not work today, for a reason
/// unrelated to #577: `resolve_codec_tag` rejects mjpeg into the `ipod`
/// muxer (`ipod cannot represent mjpeg video`) even though the system CLI
/// muxes exactly that file. That is a separate defect in the codec-tag
/// policy, not something this guard should paper over.
#[tokio::test]
async fn flac_with_cover_art_still_remuxes() {
    if !require_ffmpeg() {
        return;
    }
    assert!(
        !has_real_video(&fixtures().cover_flac),
        "fixture precondition: the cover must be an ATTACHED_PIC, not real video"
    );

    let dst = fixtures().dir.join("cover_remuxed.flac");
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    runner
        .remux(&fixtures().cover_flac, &dst, &RemuxOptions::default(), None)
        .await
        .unwrap_or_else(|e| panic!("cover art is not a video stream; must not be refused: {e:#}"));

    assert!(dst.exists(), "the remux must have produced an output file");
}

/// The carve-out, case 2: the same for an ID3v2 APIC cover in an `.mp3`
/// (`ThumbnailEmbedStrategy::Id3Apic`). A second container family, because a
/// guard could get one right by accident.
#[tokio::test]
async fn mp3_with_cover_art_still_remuxes() {
    if !require_ffmpeg() {
        return;
    }
    assert!(
        !has_real_video(&fixtures().cover_mp3),
        "fixture precondition: the cover must be an ATTACHED_PIC, not real video"
    );

    let dst = fixtures().dir.join("cover_remuxed.mp3");
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    runner
        .remux(&fixtures().cover_mp3, &dst, &RemuxOptions::default(), None)
        .await
        .unwrap_or_else(|e| panic!("cover art is not a video stream; must not be refused: {e:#}"));

    assert!(dst.exists(), "the remux must have produced an output file");
}

/// The acceptance criterion #577 was narrowed to: the behaviour is consistent
/// across ALL audio-only containers, not special-cased to `.wma`. Drives the
/// same video-bearing source into every container `video_alternative_for`
/// refuses, through the real remux path.
///
/// Deriving the matrix from the policy function rather than a hand-written
/// list is deliberate — a hand-written list is the thing that drifts, which is
/// how the deleted `ContainerFormat::is_audio_only()` ended up including
/// `Ogg`.
#[tokio::test]
async fn every_audio_only_container_refuses_the_same_source() {
    if !require_ffmpeg() {
        return;
    }
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let refused: Vec<(ContainerFormat, ContainerFormat)> = ContainerFormat::iter()
        .filter_map(|c| video_alternative_for(c).map(|alt| (c, alt)))
        .collect();
    assert_eq!(
        refused.len(),
        EXPECTED_REFUSED_CONTAINERS,
        "update EXPECTED_REFUSED_CONTAINERS if the refusal set is intentionally \
         resized: {refused:?}"
    );

    let mut executed = 0_usize;
    for (container, alternative) in refused {
        let dst = fixtures()
            .dir
            .join(format!("refused_{container}.{}", container.as_ext()));
        let _ = std::fs::remove_file(&dst);

        let Err(err) = runner
            .remux(&fixtures().video_mp4, &dst, &RemuxOptions::default(), None)
            .await
        else {
            panic!("{container} is audio-only; a video-bearing remux into it must be refused");
        };

        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains(&container.to_string().to_lowercase()),
            "{container}: error must name the target container, got: {msg}"
        );
        assert!(
            msg.contains(&alternative.to_string().to_lowercase()),
            "{container}: error must point at {alternative}, got: {msg}"
        );
        assert!(
            !dst.exists(),
            "{container}: a refusal must not leave a partial output file behind"
        );
        executed += 1;
    }
    assert_eq!(
        executed, EXPECTED_REFUSED_CONTAINERS,
        "matrix silently executed fewer cases than declared"
    );
}
