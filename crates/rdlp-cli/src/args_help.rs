//! `--help` text for the normalization flags, rendered from the owners of the
//! numbers it shows (#611): `EffectiveNormalize::PEAK_TARGET_DB` /
//! `BOOST_GAIN_DB` and `LoudnormPreset::targets()`.
//!
//! Typed literals in `args.rs` doc comments drifted from the engine before
//! (`(default: -1.0)` next to an `unwrap_or(-1.0)` two crates away), so none
//! remain: `normalization_help_numbers_come_from_the_owner` fails the build
//! if one comes back. clap 4.6 derive accepts `help = <expr>` for any
//! `impl Into<StyledStr>` — `&String` is one — so each `LazyLock<String>` is
//! attached as `help = &*HELP_X`.
//!
//! A sibling of `args.rs` rather than a block inside it because
//! `scripts/check-arg-blank-validation.sh` scans `args.rs` for field
//! declarations with a deliberately strict matcher; `format!` argument lines
//! look enough like fields to trip its residue rule.

use std::sync::LazyLock;

use rdlp_types::{EffectiveNormalize, LoudnormPreset};

pub static HELP_AUDIO_GAIN_TARGET: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Target peak level in dBFS for peak normalization (default: {:.1})",
        EffectiveNormalize::PEAK_TARGET_DB
    )
});
pub static HELP_LOUDNORM_PRESET: LazyLock<String> = LazyLock::new(|| {
    let presets = LoudnormPreset::ALL
        .iter()
        .map(|p| format!("{p} ({:.0} LUFS)", p.targets().integrated_lufs))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Loudnorm preset: {presets} [default: {}]",
        LoudnormPreset::default()
    )
});
pub static HELP_LOUDNORM_I: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Target integrated loudness in LUFS for loudnorm (default: from preset; {} = {:.1})",
        LoudnormPreset::default(),
        LoudnormPreset::default().targets().integrated_lufs
    )
});
pub static HELP_LOUDNORM_TP: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Target true peak in dBTP for loudnorm (default: from preset; {} = {:.1})",
        LoudnormPreset::default(),
        LoudnormPreset::default().targets().true_peak_dbtp
    )
});
pub static HELP_LOUDNORM_LRA: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Target loudness range in LU for loudnorm (default: from preset; {} = {:.1})",
        LoudnormPreset::default(),
        LoudnormPreset::default().targets().range_lu
    )
});
pub static HELP_NORMALIZE_BOOST: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Enable limiter-boost fallback (+{:.1} dB gain + hard limiter) for over-compressed content (implies --loudnorm)",
        EffectiveNormalize::BOOST_GAIN_DB
    )
});
pub static HELP_NORMALIZE_BOOST_DB: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Gain in dB for limiter-boost fallback (default: {:.1})",
        EffectiveNormalize::BOOST_GAIN_DB
    )
});
