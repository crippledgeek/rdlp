//! Regression test for #549: AVI remux codec-tag policy.
//!
//! `add_stream_copy` unconditionally zeroed the source codec tag for every
//! non-Matroska output. For AVI that is actively harmful:
//! - h264 (`avc1`) -> AVI: the zeroed tag lets AVI's muxer auto-fill a
//!   literal `'H264'` fourcc, which arms `avienc.c`'s start-code guard and
//!   hard-fails AVCC-packaged H.264 (no start codes in the bitstream).
//! - hevc (`hvc1`) -> AVI: AVI's tag table has NO entry for HEVC at all, so
//!   the tag stays 0 and `riff.c` silently maps a zero fourcc to raw video
//!   — exit 0, corrupt file.
//!
//! Fix (`FFmpegRunner::resolve_codec_tag`): two distinct `FFmpeg` APIs answer
//! two distinct questions. `av_codec_get_tag`/`av_codec_get_id` against the
//! muxer's own tag table decide *which tag* to write (preserve the source's
//! vs. let `FFmpeg` auto-fill) when the table has an entry for the codec.
//! `avformat_query_codec` is consulted only when the table has **no** entry
//! at all, to answer *representability* rather than tag choice — this is
//! what lets HEVC/SubRip into MKV (whose sparse, `CodecID`-keyed tag table
//! has no entry for either, but whose muxer defines `mkv_query_codec` and
//! reports both as supported) while still rejecting HEVC into AVI (whose
//! muxer defines no `query_codec` and genuinely cannot represent it). This is
//! a convention this `FFmpeg` build happens to follow, not a documented
//! container standard — no spec covers H.264/HEVC-in-AVI.
//!
//! `resolve_codec_tag` is also the decision point for the 4 raw-FFI MKV mux
//! paths (`remux_mkv_raw_ffi`, `merge_mkv_raw_ffi`'s two streams,
//! `embed_thumbnail_mkv_raw_ffi`), which previously zeroed `codec_tag`
//! directly — that asymmetry (raw-FFI always zeroed and worked;
//! `add_stream_copy` rejected on a tag-table miss) is what hid the HEVC/MKV
//! regression from this file's original matrix, since `remux_sync` routes
//! every `.mkv` target through the raw-FFI path. `mkv_raw_ffi_audio_bearing_sources_preserve_aac_and_decode`,
//! `merge_to_mkv_preserves_h264_and_aac`, and `embed_thumbnail_mkv_preserves_stream_identity`
//! below cover those 3 sites end to end.
//!
//! Requires the real `ffmpeg`/`ffprobe` CLI on `PATH` (used only to build
//! fixtures and to independently verify decode integrity + stream identity —
//! never rdlp's own code doing the checking). Fails closed: panics unless
//! `RDLP_ALLOW_SKIP_FFMPEG_TESTS` is explicitly set, so a machine missing the
//! CLI cannot silently report a green, assertion-free suite.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};

/// Number of pairings the regression matrix below is expected to exercise.
/// Asserted after the loop so a future edit that empties (or silently
/// shrinks) `cases` fails the test instead of passing vacuously.
const EXPECTED_MATRIX_CASES: usize = 8;

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

/// Require `ffmpeg`/`ffprobe` to be present. Returns `true` when the caller
/// should proceed. Panics — failing the suite — unless
/// `RDLP_ALLOW_SKIP_FFMPEG_TESTS` is explicitly set: a prior version of this
/// file `eprintln!`-and-returned on a missing CLI, which let 3 tests report
/// "passed" in 0.00s while asserting nothing (proven with stub binaries).
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

/// Build a short (1s) fixture with the given video encoder and container.
/// Panics (rather than returning `None`) if the build fails — a fixture that
/// silently fails to build must not silently shrink the test matrix.
fn build_fixture(dir: &Path, stem: &str, ext: &str, vcodec: &str) -> PathBuf {
    let path = dir.join(format!("{stem}.{ext}"));
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "testsrc=d=1:s=320x240:r=25"])
        .args(["-f", "lavfi", "-i", "sine=d=1"])
        .args(["-c:v", vcodec, "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "64k", "-shortest"])
        .arg(&path)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn ffmpeg for {stem}.{ext} fixture: {e}"));
    assert!(
        status.success(),
        "ffmpeg fixture build failed for {stem}.{ext} (vcodec={vcodec}); \
         is {vcodec} compiled into this FFmpeg build?"
    );
    path
}

/// True if `ffmpeg -f null` can decode every frame of `path` without error.
/// An independent check via the system CLI, never rdlp's own decode path.
fn decodes_cleanly(path: &Path) -> bool {
    Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .map(|o| o.status.success() && o.stderr.is_empty())
        .unwrap_or(false)
}

/// The first video stream's codec identity, per `ffprobe` — an independent
/// check that distinguishes a correctly-tagged stream from a
/// mistagged-but-decodable one. This is the exact corruption mode #549
/// introduced: AVI's zeroed HEVC tag decodes cleanly as `rawvideo` (`riff.c`
/// maps a zero fourcc to raw video), so `decodes_cleanly` alone cannot catch
/// it — only asserting the decoded codec name is what it should be can.
#[derive(Debug)]
struct StreamIdentity {
    codec_name: String,
    codec_tag_string: String,
}

/// `selector` is an ffprobe stream selector (`"v:0"`, `"a:0"`).
fn probe_stream(path: &Path, selector: &str) -> StreamIdentity {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", selector])
        .args(["-show_entries", "stream=codec_name,codec_tag_string"])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn ffprobe on {}: {e}", path.display()));
    assert!(
        output.status.success(),
        "ffprobe failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut fields = stdout.trim().split(',');
    let codec_name = fields
        .next()
        .unwrap_or_else(|| panic!("no codec_name field for {} ({selector})", path.display()))
        .to_string();
    let codec_tag_string = fields
        .next()
        .unwrap_or_else(|| {
            panic!(
                "no codec_tag_string field for {} ({selector})",
                path.display()
            )
        })
        .to_string();
    StreamIdentity {
        codec_name,
        codec_tag_string,
    }
}

fn probe_video_stream(path: &Path) -> StreamIdentity {
    probe_stream(path, "v:0")
}

fn probe_audio_stream(path: &Path) -> StreamIdentity {
    probe_stream(path, "a:0")
}

/// Build a video-only h264 elementary source (no audio stream) — needed by
/// `merge`, which takes separate video and audio inputs.
fn build_video_only(dir: &Path) -> PathBuf {
    let path = dir.join("video_only.mp4");
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "testsrc=d=1:s=320x240:r=25"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&path)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn ffmpeg for video_only fixture: {e}"));
    assert!(status.success(), "video_only fixture build failed");
    path
}

/// Build an audio-only AAC elementary source (no video stream).
fn build_audio_only(dir: &Path) -> PathBuf {
    let path = dir.join("audio_only.m4a");
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "sine=d=1"])
        .args(["-c:a", "aac", "-b:a", "64k"])
        .arg(&path)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn ffmpeg for audio_only fixture: {e}"));
    assert!(status.success(), "audio_only fixture build failed");
    path
}

/// Build a tiny still JPEG cover image, for thumbnail-embed tests.
fn build_cover_jpg(dir: &Path) -> PathBuf {
    let path = dir.join("cover.jpg");
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "color=c=red:s=64x64"])
        .args(["-frames:v", "1"])
        .arg(&path)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn ffmpeg for cover.jpg fixture: {e}"));
    assert!(status.success(), "cover.jpg fixture build failed");
    path
}

/// Fixtures shared across every test in this file, built once (an `ffmpeg`
/// subprocess per source is real work — this is the "cached, not
/// regenerated per test" fixture idiom used across this crate's test suite).
struct Fixtures {
    dir: PathBuf,
    h264_mp4: PathBuf,
    hevc_mp4: PathBuf,
    ts: PathBuf,
    mkv: PathBuf,
    video_only: PathBuf,
    audio_only: PathBuf,
    cover_jpg: PathBuf,
}

static FIXTURES: OnceLock<Fixtures> = OnceLock::new();

/// Fixtures are built once per process and cached, so they live under the
/// crate's `target/` rather than in a `TempDir`.
///
/// A `static` is never dropped, so a `TempDir` here could never run its
/// cleanup — it would leak the whole fixture set (~448K of built video and
/// image files) on every run, pass or panic. That matters because this
/// machine's `/tmp` is a RAM-backed tmpfs, so the leak would be real memory.
/// Building into `target/` instead makes the residue ordinary build output:
/// it costs no RAM, `cargo clean` reclaims it, and a rebuilt fixture simply
/// overwrites the previous one, which also makes repeat runs faster.
fn fixture_dir() -> PathBuf {
    // CARGO_TARGET_TMPDIR is `<target>/tmp`, provided by cargo for integration
    // tests exactly like this one.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("remux_avi_codec_tag");
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| panic!("create fixture dir {}: {e}", dir.display()));
    dir
}

fn fixtures() -> &'static Fixtures {
    FIXTURES.get_or_init(|| {
        let dir = fixture_dir();
        let h264_mp4 = build_fixture(&dir, "h264", "mp4", "libx264");
        let hevc_mp4 = build_fixture(&dir, "hevc", "mp4", "libx265");
        let ts = build_fixture(&dir, "src", "ts", "libx264");
        let mkv = build_fixture(&dir, "src", "mkv", "libx264");
        let video_only = build_video_only(&dir);
        let audio_only = build_audio_only(&dir);
        let cover_jpg = build_cover_jpg(&dir);
        Fixtures {
            dir,
            h264_mp4,
            hevc_mp4,
            ts,
            mkv,
            video_only,
            audio_only,
            cover_jpg,
        }
    })
}

/// Remux `src` to `<label>.<target_ext>` and assert: the output decodes
/// cleanly, its video codec identity matches `expected_codec`, and — when
/// given — its `codec_tag_string` matches `expected_tag`. A mutation that
/// flips `CodecTagAction::Preserve` <-> `Clear` cannot survive the
/// `expected_tag` assertion: the stream would still decode (mistagged as
/// something FFmpeg auto-fills), but under the wrong tag.
async fn assert_remux_matches(
    runner: &FFmpegRunner,
    src: &Path,
    target_ext: &str,
    label: &str,
    expected_codec: &str,
    expected_tag: Option<&str>,
) {
    let dst = fixtures().dir.join(format!("{label}.{target_ext}"));
    runner
        .remux(src, &dst, &RemuxOptions::default(), None)
        .await
        .unwrap_or_else(|e| panic!("{label}: remux failed: {e:#}"));
    assert!(
        decodes_cleanly(&dst),
        "{label}: output does not decode cleanly"
    );

    let identity = probe_video_stream(&dst);
    assert_eq!(
        identity.codec_name, expected_codec,
        "{label}: decoded codec mismatch (mistagged-but-decodable is exactly \
         the #549 corruption mode) — got {identity:?}"
    );
    if let Some(tag) = expected_tag {
        assert_eq!(
            identity.codec_tag_string, tag,
            "{label}: codec_tag_string mismatch — got {identity:?}"
        );
    }
}

/// The positive case: h264 must remux into AVI, decode as h264, and keep its
/// source `avc1` tag. Fails against unpatched code (`avienc.c`'s start-code
/// guard rejects the muxer-auto-filled `'H264'` tag on AVCC-packaged H.264).
#[tokio::test]
async fn h264_mp4_to_avi_preserves_tag_and_decodes() {
    if !require_ffmpeg() {
        return;
    }
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");
    assert_remux_matches(
        &runner,
        &fixtures().h264_mp4,
        "avi",
        "h264_to_avi",
        "h264",
        Some("avc1"),
    )
    .await;
}

/// The negative case: HEVC cannot be represented in AVI (this build's tag
/// table has no entry for it, and `avienc` defines no `query_codec` fallback)
/// — the remux must be REJECTED with an actionable message and must not
/// leave a corrupt (rawvideo-tagged) or empty output file behind. Fails
/// against unpatched code, which either returns `Ok(())` and leaves a
/// corrupt file with `AV_CODEC_ID_RAWVIDEO` on read-back (the original bug),
/// or rejects but leaves a 0-byte file (the pre-fix behavior of this predicate).
#[tokio::test]
async fn hevc_mp4_to_avi_is_rejected_with_actionable_message() {
    if !require_ffmpeg() {
        return;
    }
    let dst = fixtures().dir.join("hevc_to_avi.avi");
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let err = runner
        .remux(&fixtures().hevc_mp4, &dst, &RemuxOptions::default(), None)
        .await
        .expect_err("AVI cannot represent HEVC; the remux must be rejected");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("avi") && msg.contains("hevc"),
        "error should name the container/codec pairing, got: {msg}"
    );
    assert!(
        !dst.exists(),
        "rejection must not leave a partial/empty output file at {}",
        dst.display()
    );
}

/// IMPORTANT #2 (code-review follow-up): the 0-byte-output cleanup on a
/// codec-tag rejection previously landed only at the `remux.rs` call site;
/// `embed_metadata` shares the exact same `format::output` (create/truncate)
/// -then-`add_stream_copy` (can newly fail) shape and had no cleanup at all.
/// Reuses the proven HEVC-into-AVI rejection (same as
/// `hevc_mp4_to_avi_is_rejected_with_actionable_message` above) through a
/// different call site to prove the cleanup helper landed there too.
#[tokio::test]
async fn metadata_embed_hevc_into_avi_leaves_no_partial_file() {
    if !require_ffmpeg() {
        return;
    }
    let dst = fixtures().dir.join("hevc_metadata_to_avi.avi");
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let err = runner
        .embed_metadata(
            &fixtures().hevc_mp4,
            &dst,
            &std::collections::HashMap::new(),
            &[],
            None,
            None,
        )
        .await
        .expect_err("AVI cannot represent HEVC; metadata embed must be rejected too");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("avi") && msg.contains("hevc"),
        "error should name the container/codec pairing, got: {msg}"
    );
    assert!(
        !dst.exists(),
        "rejection must not leave a partial/empty output file at {}",
        dst.display()
    );
}

/// Regression guard matrix: every pairing that worked before this fix must
/// keep working — the MKV raw-FFI path (now also routed through
/// `resolve_codec_tag`, see `mkv_raw_ffi_audio_bearing_sources_preserve_aac_and_decode`
/// below for the audio-stream-specific check that change needed) and every
/// non-AVI, non-tag-rejecting target reachable from the generic remux path.
/// Also covers the CRITICAL regression this predicate must not reintroduce:
/// HEVC into MKV via the generic (non-raw-FFI) path must succeed, not just
/// the MKV-via-raw-FFI path `remux_sync` normally routes to.
#[tokio::test]
async fn remux_regression_matrix_still_decodes() {
    if !require_ffmpeg() {
        return;
    }
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let cases: &[(&str, &Path, &str, &str)] = &[
        ("mp4_to_mkv", &fixtures().h264_mp4, "mkv", "h264"),
        ("ts_to_mkv", &fixtures().ts, "mkv", "h264"),
        ("hevc_to_mkv", &fixtures().hevc_mp4, "mkv", "hevc"),
        ("mp4_to_mp4", &fixtures().h264_mp4, "mp4", "h264"),
        ("mp4_to_mov", &fixtures().h264_mp4, "mov", "h264"),
        ("mp4_to_flv", &fixtures().h264_mp4, "flv", "h264"),
        ("mp4_to_ts", &fixtures().h264_mp4, "ts", "h264"),
        ("mkv_to_mp4", &fixtures().mkv, "mp4", "h264"),
    ];
    assert_eq!(
        cases.len(),
        EXPECTED_MATRIX_CASES,
        "update EXPECTED_MATRIX_CASES if this matrix is intentionally resized"
    );

    let mut executed = 0usize;
    for (label, src, target_ext, expected_codec) in cases {
        assert_remux_matches(&runner, src, target_ext, label, expected_codec, None).await;
        executed += 1;
    }
    assert_eq!(
        executed, EXPECTED_MATRIX_CASES,
        "matrix silently executed fewer cases than declared"
    );
}

/// CRITICAL regression guard (#1/#2): `remux_sync` branches HEVC/MKV away to
/// `remux_mkv_raw_ffi` *before* it ever reaches `add_stream_copy`, so the
/// matrix above never actually drives `resolve_codec_tag` for an MKV target.
/// `embed_metadata` copies streams via `add_stream_copy` with no such
/// branch, so it is the vehicle here: an HEVC source into an MKV target must
/// succeed. Fails against a predicate that rejects whenever the muxer's tag
/// table has no entry for the codec (MKV's sparse, `CodecID`-keyed table has
/// none for HEVC) instead of falling back to `avformat_query_codec`.
#[tokio::test]
async fn hevc_mp4_to_mkv_via_add_stream_copy_succeeds() {
    if !require_ffmpeg() {
        return;
    }
    let dst = fixtures().dir.join("hevc_via_metadata.mkv");
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    runner
        .embed_metadata(
            &fixtures().hevc_mp4,
            &dst,
            &std::collections::HashMap::new(),
            &[],
            None,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("HEVC into MKV via add_stream_copy must succeed: {e:#}"));

    assert!(
        decodes_cleanly(&dst),
        "hevc_via_metadata: output does not decode cleanly"
    );
    let identity = probe_video_stream(&dst);
    assert_eq!(identity.codec_name, "hevc", "got {identity:?}");
}

/// Empirical check for routing `remux_mkv_raw_ffi` through `resolve_codec_tag`
/// (it previously always zeroed `codec_tag` directly): every fixture in this
/// file carries AAC audio tagged `mp4a` (the MP4/ISOBMFF fourcc, per
/// `build_fixture`'s `-c:a aac`), and MKV's tag table *does* have an AAC
/// entry — just not under `mp4a` — so `resolve_codec_tag` must resolve that
/// stream to `Clear`, not `Preserve`. A prior, cruder experiment that
/// preserved every source tag unconditionally regressed exactly these
/// AAC-bearing mkv targets (`mp4_to_mkv`/`ts_to_mkv`/`hevc_to_mkv` all went
/// from OK to FAIL). This test would fail the same way if that regression
/// reappeared: both audio and video must decode, and ffprobe must report the
/// audio stream as `aac`, not corrupt/misidentified.
#[tokio::test]
async fn mkv_raw_ffi_audio_bearing_sources_preserve_aac_and_decode() {
    if !require_ffmpeg() {
        return;
    }
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let cases: &[(&str, &Path, &str)] = &[
        ("audio_mp4_to_mkv", &fixtures().h264_mp4, "h264"),
        ("audio_ts_to_mkv", &fixtures().ts, "h264"),
        ("audio_hevc_to_mkv", &fixtures().hevc_mp4, "hevc"),
    ];

    for (label, src, expected_video_codec) in cases {
        let dst = fixtures().dir.join(format!("{label}.mkv"));
        runner
            .remux(src, &dst, &RemuxOptions::default(), None)
            .await
            .unwrap_or_else(|e| panic!("{label}: remux failed: {e:#}"));
        assert!(
            decodes_cleanly(&dst),
            "{label}: output does not decode cleanly"
        );

        let video = probe_video_stream(&dst);
        assert_eq!(
            video.codec_name, *expected_video_codec,
            "{label}: {video:?}"
        );

        let audio = probe_audio_stream(&dst);
        assert_eq!(
            audio.codec_name, "aac",
            "{label}: audio stream mistagged/misdecoded — got {audio:?}"
        );
    }
}

/// `merge_mkv_raw_ffi`'s video and audio streams both now resolve their
/// codec tag through `resolve_codec_tag` instead of always zeroing. Merges
/// separate h264 video-only and aac audio-only sources into MKV and checks
/// both stream identities.
#[tokio::test]
async fn merge_to_mkv_preserves_h264_and_aac() {
    if !require_ffmpeg() {
        return;
    }
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");
    let dst = fixtures().dir.join("merged.mkv");

    runner
        .merge(
            &fixtures().video_only,
            &fixtures().audio_only,
            &dst,
            &RemuxOptions::default(),
            None,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("merge into mkv must succeed: {e:#}"));

    assert!(
        decodes_cleanly(&dst),
        "merged output does not decode cleanly"
    );
    let video = probe_video_stream(&dst);
    assert_eq!(video.codec_name, "h264", "got {video:?}");
    let audio = probe_audio_stream(&dst);
    assert_eq!(audio.codec_name, "aac", "got {audio:?}");
}

/// `embed_thumbnail_mkv_raw_ffi`'s media-stream loop also now resolves its
/// codec tag through `resolve_codec_tag`. Embeds a cover image into an
/// h264+aac source remuxed to MKV and checks both media stream identities
/// survive alongside the attachment.
#[tokio::test]
async fn embed_thumbnail_mkv_preserves_stream_identity() {
    if !require_ffmpeg() {
        return;
    }
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");
    let dst = fixtures().dir.join("with_thumb.mkv");

    runner
        .embed_thumbnail(
            &fixtures().h264_mp4,
            &fixtures().cover_jpg,
            &dst,
            "mkv",
            None,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("thumbnail embed into mkv must succeed: {e:#}"));

    assert!(
        decodes_cleanly(&dst),
        "thumbnail-embedded output does not decode cleanly"
    );
    let video = probe_video_stream(&dst);
    assert_eq!(video.codec_name, "h264", "got {video:?}");
    let audio = probe_audio_stream(&dst);
    assert_eq!(audio.codec_name, "aac", "got {audio:?}");
}
