//! Verifies that `build.rs` bakes the resolved FFmpeg prefix into the binary
//! via `cargo:rustc-env=RDLP_FFMPEG_PREFIX=...`.
//!
//! See PR E-1 (FFmpeg linkage visibility) — runtime diagnostics in PR E-3 will
//! read this env to surface the linkage to end users.
//!
//! Scope: this is a deliberate smoke check that the env is emitted at all
//! (the load-bearing precondition for reading it via `env!()`). Branch coverage
//! for the broken-prefix detection logic lives in `tests/pkgconfig_intent.rs`.
//!
//! Whether the *linked* FFmpeg matches what these bindings were generated
//! against is asserted at run time by `ffmpeg::abi`, called from
//! `ensure_init()` (rdlp#656). That assertion used to be deferred here to an
//! `rdlp doctor` command (PR E-2) which does not exist; the failure is silent,
//! and nobody runs a diagnostic for a bug they cannot see. The prefix baked by
//! this env is the *message* half of that check, never the comparison.

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
