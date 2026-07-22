//! Regression test for #629: MXF output needs a frame rate on the output
//! *stream*, not only on the encoder context.
//!
//! `mxf_init` (libavformat/mxfenc.c) derives the container's edit rate from
//! the AVStream, in this order:
//!
//! ```c
//! AVRational tbc = (AVRational){ 0, 0 };
//! if (st->avg_frame_rate.num > 0 && st->avg_frame_rate.den > 0)
//!     tbc = av_inv_q(st->avg_frame_rate);
//! else if (st->r_frame_rate.num > 0 && st->r_frame_rate.den > 0)
//!     tbc = av_inv_q(st->r_frame_rate);
//! ```
//!
//! then rejects an unrepresentable rate in `mxf_init_timecode`:
//! `"Unsupported frame rate %d/%d. Set -strict option to 'unofficial' or
//! lower in order to allow it!"`. Note it never consults `st->time_base`,
//! which is the only field rdlp used to set — so `tbc` stayed `0/0` and
//! `--recode-video=mxf` failed at `write_header` for **every** encoder, and
//! the plain remux path failed for the same reason.
//!
//! MXF is the only muxer rdlp targets that demands this; every other one
//! infers a workable rate, which is why the gap went unnoticed until #618's
//! end-to-end mux pass.
//!
//! Requires the real `ffmpeg`/`ffprobe` CLI on `PATH` to build fixtures and to
//! verify the result independently of rdlp's own decode path. Fails closed:
//! panics unless `RDLP_ALLOW_SKIP_FFMPEG_TESTS` is set, so a machine without
//! the CLI cannot report a green, assertion-free run.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions, VideoConvertOptions};

/// The fixture's frame rate, and therefore the rate both output paths must
/// carry into the MXF edit rate. 25 fps is one of the rates
/// `ff_mxf_get_content_package_rate` accepts, so a correct implementation
/// cannot fail this test for being an exotic rate.
const FIXTURE_FPS: u32 = 25;

fn ffmpeg_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

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
         fixtures and independently verify output (never rdlp's own decode path). \
         Set RDLP_ALLOW_SKIP_FFMPEG_TESTS=1 to explicitly opt into skipping it."
    );
}

/// An h264 + 48 kHz PCM fixture at [`FIXTURE_FPS`].
///
/// Both audio properties are MXF requirements, not arbitrary choices, and
/// getting either wrong fails `write_header` with `EPERM` — which looks
/// exactly like the #629 failure this file is about, so it would send a
/// future reader chasing the wrong bug:
///
/// - **48 kHz only.** `mxf_write_header` refuses anything else outright:
///   "only 48khz is implemented". `ffmpeg -c copy` on a 44.1 kHz source fails
///   the same way, so this is the container, not rdlp.
/// - **PCM S16LE/S24LE only.** OP1a carries no AAC, so the fixture is built
///   with PCM rather than transcoded from it.
fn build_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("src.mov");
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc=d=1:s=320x240:r={FIXTURE_FPS}"),
        ])
        .args(["-f", "lavfi", "-i", "sine=d=1:r=48000"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "pcm_s16le", "-ar", "48000", "-shortest"])
        .arg(&path)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn ffmpeg for fixture: {e}"));
    assert!(status.success(), "ffmpeg fixture build failed");
    path
}

/// The first video stream's `r_frame_rate` per ffprobe, as `(num, den)`.
/// Read back through the CLI rather than rdlp's probe so the assertion is
/// independent of the code under test.
fn probed_frame_rate(path: &Path) -> (u32, u32) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=r_frame_rate"])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn ffprobe on {}: {e}", path.display()));
    assert!(out.status.success(), "ffprobe failed on {}", path.display());
    let raw = String::from_utf8_lossy(&out.stdout);
    let raw = raw.trim();
    let (num, den) = raw
        .split_once('/')
        .unwrap_or_else(|| panic!("unexpected r_frame_rate {raw:?}"));
    (num.parse().unwrap(), den.parse().unwrap())
}

/// Frames that actually decode out of the first video stream.
///
/// Deliberately *not* the usual "`ffmpeg -f null -` exits 0 with empty
/// stderr" check this crate's other tests use: run against MXF, that check
/// fails on `ffmpeg`'s **own** output. Muxing the fixture with
/// `ffmpeg -c copy` and decoding the result emits
///
/// ```text
/// [null @ ...] Application provided invalid, non monotonically increasing
/// dts to muxer in stream 0: 24 >= 23
/// ```
///
/// which comes from the null *muxer* re-timestamping MXF's constant edit
/// rate, not from any decode failure. A check whose control fails proves
/// nothing about the subject, so integrity is asserted ordinally instead:
/// every source frame must come back out.
fn decoded_frame_count(path: &Path) -> u32 {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=nb_read_frames"])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn ffprobe on {}: {e}", path.display()));
    assert!(out.status.success(), "ffprobe failed on {}", path.display());
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("unparsable frame count for {}: {e}", path.display()))
}

/// The transcode path: `--recode-video=mxf` with an explicit encoder.
#[tokio::test]
async fn transcode_into_mxf_carries_the_frame_rate() {
    if !require_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = build_fixture(dir.path());
    let output = dir.path().join("out.mxf");

    let runner = FFmpegRunner::new().expect("ffmpeg init");
    let opts = VideoConvertOptions {
        remux_only: false,
        video_codec: Some(rdlp_types::media_name::VideoEncoderName::from_static(
            "libx264",
        )),
        audio_copy: false,
        audio_codec: Some(rdlp_types::media_name::AudioEncoderName::from_static(
            "pcm_s16le",
        )),
        ..Default::default()
    };
    runner
        .convert_video(&input, &output, &opts, None, None, None)
        .await
        .expect("mxf transcode must succeed: mxf_init needs a non-zero stream frame rate");

    assert_eq!(
        probed_frame_rate(&output),
        (FIXTURE_FPS, 1),
        "MXF edit rate must match the source"
    );
    assert_eq!(
        decoded_frame_count(&output),
        decoded_frame_count(&input),
        "every source frame must decode out of the MXF"
    );
}

/// The stream-copy path. #629's body describes only the transcode failure,
/// but a plain `h264 -> mxf` remux fails identically and for the same reason:
/// frame rates live on `AVStream`, not in `AVCodecParameters`, so
/// `avcodec_parameters_copy` never carried them.
#[tokio::test]
async fn remux_into_mxf_carries_the_frame_rate() {
    if !require_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = build_fixture(dir.path());
    let output = dir.path().join("copy.mxf");

    let runner = FFmpegRunner::new().expect("ffmpeg init");
    runner
        .remux(&input, &output, &RemuxOptions::default(), None)
        .await
        .expect("mxf remux must succeed");

    assert_eq!(
        probed_frame_rate(&output),
        (FIXTURE_FPS, 1),
        "MXF edit rate must survive a stream copy"
    );
    assert_eq!(
        decoded_frame_count(&output),
        decoded_frame_count(&input),
        "every source frame must survive the stream copy"
    );
}
