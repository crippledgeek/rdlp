//! Shared fixtures and probes for the `rdlp-ffmpeg` integration suites.
//!
//! `ffmpeg_available()` was hand-copied into 16 of the 22 test files in this
//! directory, and 11 of them hand-roll their own `ffprobe` invocation. This
//! module is the single definition, mirroring `rdlp-postprocess/tests/common`.
//! New suites should use it; the pre-existing copies are migrated
//! opportunistically rather than in one sweep (#640).
//!
//! The system `ffmpeg`/`ffprobe` CLIs are used ONLY to build fixtures and to
//! verify results — production code never spawns a subprocess (the
//! library-first rule in `CLAUDE.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    dead_code
)]

use std::path::Path;
use std::process::Command;

/// Whether the system `ffmpeg` CLI is usable.
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether the linked `FFmpeg` build provides `name` as an encoder.
///
/// The custom `mediaforge` build and a distro build differ in codec coverage,
/// so a missing encoder must skip a test rather than fail it.
pub fn encoder_available(name: &str) -> bool {
    ffmpeg_the_third::encoder::find_by_name(name).is_some()
}

/// Read a single `ffprobe` stream field from the first audio stream.
///
/// Returns `None` when `ffprobe` fails or the field is empty — which for a
/// truncated or unmuxable output is the answer the caller wants.
pub fn probe_audio_field(path: &Path, field: &str) -> Option<String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            &format!("stream={field}"),
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

/// Count decoded frames by actually decoding the file.
///
/// `select` is an ffprobe stream specifier (`"a:0"`, `"v:0"`); `None` counts
/// across every stream, which is what a "did this file survive the mux at all"
/// check wants.
///
/// Decoding rather than stat-ing is deliberate: a failed mux still leaves a
/// non-empty partial header on disk, so `len() > 0` scores a failure as a
/// success — the verification trap that produced a wrong answer twice while
/// diagnosing #637/#638.
pub fn decoded_frames(path: &Path, select: Option<&str>) -> Option<u64> {
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error"]);
    if let Some(spec) = select {
        cmd.args(["-select_streams", spec]);
    }
    let out = cmd
        .args([
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // With no selector ffprobe emits one line per stream; the file decoded if
    // any stream yielded frames.
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<u64>().ok())
        .max()
}

/// Audio-stream frame count. Thin alias over [`decoded_frames`] for the
/// suites that only care about `a:0`.
pub fn decoded_audio_frames(path: &Path) -> Option<u64> {
    decoded_frames(path, Some("a:0"))
}
