// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs
)]

//! E2E test: `process_local_file` must NOT delete the user's source file.
//!
//! Industry standard: transform tools (ffmpeg, HandBrake, ImageMagick) never
//! delete a user-supplied input by default. This test pins that invariant for
//! `RdlpClient::process_local_file` (#414).
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! input fixture).

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

use rdlp_api::RdlpClient;
use rdlp_types::{Config, ContainerFormat, PostProcess};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a tiny (2 s) H.264 yuv420p MP4 fixture — fast enough for a success
/// path test.
///
/// # Panics
/// Panics (with captured stderr) if ffmpeg is present but the fixture-build
/// command exits non-zero. A broken ffmpeg (e.g. missing libx264) is a real
/// problem and must not silently pass the test.
fn build_short_mp4_fixture(dir: &Path) -> std::path::PathBuf {
    let src = dir.join("source.mp4");
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=2:s=320x240",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            src.to_str().unwrap(),
        ])
        .output()
        .expect("failed to spawn ffmpeg");
    if !out.status.success() {
        panic!(
            "ffmpeg fixture-build failed (exit {})\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    src
}

/// `process_local_file` must preserve the user's source MP4 after a successful
/// remux to MKV. Before the #414 fix (`keep_inputs=false`), the pipeline's
/// `replace()` moved the source into the delete-set and `cleanup()` deleted it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_process_local_file_keeps_source_on_success() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg not available");
        return;
    }
    let dir = TempDir::new().expect("tempdir");
    let src = build_short_mp4_fixture(dir.path());

    // Config: remux to MKV so the pipeline produces `source.mkv` distinct from
    // `source.mp4`. This forces at least one pipeline stage (RemuxStage) to run
    // and exercise the replace() → cleanup() path.
    let config = Config {
        progress: false,
        postprocess: PostProcess {
            remux_container: Some(ContainerFormat::Mkv),
            ..PostProcess::default()
        },
        ..Default::default()
    };
    let client = RdlpClient::new(config).expect("client should build");

    let src_clone = src.clone();
    let mut handle = client.process_local_file(src_clone);

    // Drain all events.
    while let Some(_event) = handle.events().recv().await {}

    let result = handle.wait().await;

    // The source must survive regardless of whether the pipeline succeeded.
    assert!(
        src.exists(),
        "process_local_file must NOT delete the user's source file (#414); source.mp4 is gone"
    );

    // The pipeline must have succeeded and produced an MKV output.
    let output_files = result.expect("process_local_file should succeed");
    assert!(
        !output_files.output_files.is_empty(),
        "Expected at least one output file"
    );
    let mkv_output = output_files
        .output_files
        .iter()
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mkv")));
    assert!(
        mkv_output.is_some(),
        "Expected an MKV output file; got: {:?}",
        output_files.output_files
    );
    assert!(
        mkv_output.unwrap().exists(),
        "MKV output file does not exist on disk"
    );
}
