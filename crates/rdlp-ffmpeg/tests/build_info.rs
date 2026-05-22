//! Verifies that `build.rs` bakes the resolved FFmpeg prefix into the binary
//! via `cargo:rustc-env=RDLP_FFMPEG_PREFIX=...`.
//!
//! See PR E-1 (FFmpeg linkage visibility) — future runtime diagnostics in
//! PR E-2 / E-3 will read this env to surface the linkage to end users.

#[test]
fn ffmpeg_prefix_baked_into_binary() {
    // build.rs in this crate must emit cargo:rustc-env=RDLP_FFMPEG_PREFIX=...
    // even if empty, so consumers can read it via env!().
    // A None here means build.rs didn't run / didn't emit the env.
    let prefix = option_env!("RDLP_FFMPEG_PREFIX");
    assert!(
        prefix.is_some(),
        "RDLP_FFMPEG_PREFIX env was not baked into the binary at build time. \
         Check that crates/rdlp-ffmpeg/build.rs is present and emitting \
         cargo:rustc-env=RDLP_FFMPEG_PREFIX=..."
    );
}
