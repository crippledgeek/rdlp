//! Code-review follow-up to #549: `salvage_remux_sync` copies every stream in
//! its input with no media-type filter (unlike `remux`/`metadata`/`fixup`,
//! which restrict to Video|Audio) — by design, since salvage is the
//! corruption-recovery path of last resort and must not silently drop an
//! attachment.
//!
//! Before the medium gate in `resolve_codec_tag`, an attachment stream was put
//! to a *codec representability* question it was never meant to answer (FFmpeg
//! muxes attachments outside the codec path entirely), so an MKV carrying a
//! font attachment — the standard layout for a subtitled release — made
//! `add_stream_copy` return `IncompatibleContainerCodec { codec: "ttf", .. }`
//! and salvage failed outright on exactly the input it exists to recover.
//!
//! Verifies via `ffprobe`, independent of rdlp's own decode path, that the
//! attachment SURVIVES the salvage remux — not merely that `salvage_remux_sync`
//! returns `Ok`. A second latent defect surfaced while proving this: Matroska's
//! `mkv_write_attachments` hard-requires a `mimetype` tag, so the codec-tag fix
//! alone still failed at `write_header` until salvage copied per-stream
//! metadata.
//!
//! Lives under `tests/` deliberately: `scripts/check-no-cli.sh` forbids
//! `std::process::Command` anywhere in `crates/*/src/` (the library-first
//! principle) and scans production sources only, so CLI-driven fixtures are
//! sanctioned here and nowhere else.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rdlp_ffmpeg::ffmpeg::salvage::salvage_remux_sync;

/// Fails closed. A missing CLI must break the run rather than silently turn
/// this regression guard into a no-op that still reports green — the exact
/// defect code review flagged in this branch's other test file. Set
/// `RDLP_ALLOW_SKIP_FFMPEG_TESTS` to skip deliberately.
fn require_cli() -> bool {
    let have = ["ffmpeg", "ffprobe"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    });
    assert!(
        have || std::env::var_os("RDLP_ALLOW_SKIP_FFMPEG_TESTS").is_some(),
        "ffmpeg/ffprobe are required for the #549 salvage regression test; \
         set RDLP_ALLOW_SKIP_FFMPEG_TESTS to skip deliberately"
    );
    have
}

/// A real MKV with video, audio and a native Matroska font attachment. The
/// attached bytes need not be a genuine TTF: `-attach` embeds them verbatim
/// with the given mimetype/filename metadata regardless of content. The
/// audio stream also carries a `language` tag, unrelated to the attachment,
/// so a test can pin that `salvage_remux_sync`'s per-stream metadata copy
/// (`salvage.rs`) covers every stream — not just the attachment it was added
/// to fix.
fn build_font_attached_mkv(dir: &Path) -> PathBuf {
    let font_path = dir.join("font.ttf");
    std::fs::write(&font_path, b"\x00\x01\x00\x00not a real font, just bytes")
        .expect("write fake font fixture");

    let media_path = dir.join("with_font.mkv");
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "testsrc=d=1:s=320x240:r=25"])
        .args(["-f", "lavfi", "-i", "sine=d=1"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "64k", "-shortest"])
        .args(["-metadata:s:a:0", "language=jpn"])
        .arg("-attach")
        .arg(&font_path)
        .args(["-metadata:s:t", "mimetype=application/x-truetype-font"])
        .arg(&media_path)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn ffmpeg for font-attached mkv fixture: {e}"));
    assert!(status.success(), "font-attached mkv fixture build failed");
    media_path
}

fn ffprobe_entries(path: &Path, args: &[&str]) -> String {
    let output = Command::new("ffprobe")
        .args(["-v", "error"])
        .args(args)
        .arg(path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn ffprobe on {}: {e}", path.display()));
    assert!(
        output.status.success(),
        "ffprobe failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The CRITICAL regression: salvage must succeed on a font-attached input and
/// the salvaged output must still carry that attachment. Fails against the
/// unpatched `resolve_codec_tag`, which applied its representability rejection
/// uniformly across every `AVMediaType`.
#[test]
fn salvage_preserves_font_attachment() {
    if !require_cli() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let input = build_font_attached_mkv(dir.path());

    let salvaged = salvage_remux_sync(&input)
        .unwrap_or_else(|e| panic!("salvage must succeed on a font-attached input: {e:#}"));

    let tags = ffprobe_entries(
        &salvaged,
        &[
            "-select_streams",
            "t",
            "-show_entries",
            "stream_tags=mimetype,filename",
            "-of",
            "default=noprint_wrappers=1",
        ],
    );
    assert!(
        tags.contains("mimetype=application/x-truetype-font"),
        "salvaged output lost the font attachment's mimetype tag: {tags}"
    );
    assert!(
        tags.contains("filename=font.ttf"),
        "salvaged output lost the font attachment's filename tag: {tags}"
    );

    // The video/audio streams must have survived alongside the attachment —
    // salvage must not trade one stream for another.
    let names = ffprobe_entries(
        &salvaged,
        &["-show_entries", "stream=codec_name", "-of", "csv=p=0"],
    );
    for expected in ["h264", "aac", "ttf"] {
        assert!(
            names.contains(expected),
            "{expected} stream missing: {names}"
        );
    }
}

/// IMPORTANT #6 (code-review follow-up): `salvage_remux_sync`'s per-stream
/// metadata copy (added to fix the attachment's `mimetype` requirement) runs
/// unconditionally for every stream, not just attachments. This pins the
/// non-attachment side: the audio stream's `language` tag — set on the input
/// fixture, unrelated to the attachment fix — must also survive.
#[test]
fn salvage_preserves_non_attachment_stream_metadata() {
    if !require_cli() {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let input = build_font_attached_mkv(dir.path());

    let salvaged = salvage_remux_sync(&input)
        .unwrap_or_else(|e| panic!("salvage must succeed on a font-attached input: {e:#}"));

    let tags = ffprobe_entries(
        &salvaged,
        &[
            "-select_streams",
            "a",
            "-show_entries",
            "stream_tags=language",
            "-of",
            "default=noprint_wrappers=1",
        ],
    );
    assert!(
        tags.contains("TAG:language=jpn"),
        "salvaged output lost the audio stream's language tag: {tags}"
    );
}
