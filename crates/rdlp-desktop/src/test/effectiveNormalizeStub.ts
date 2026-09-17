// A stub `EffectiveNormalize` payload for section tests.
//
// Every value is deliberately DISTINCT from the engine's real defaults (which
// live in `rdlp_types::EffectiveNormalize` / `LoudnormPreset::targets` and are
// not restated here) and from every other field, so a placeholder assertion
// can only pass if the section really derives it from this payload — a
// leftover literal, or a field wired to the wrong key, fails.

import type { EffectiveNormalize, LoudnormPresetInfo } from "@/types";

export const effectiveNormalizeStub: EffectiveNormalize = {
    preset: "broadcast",
    targets: { integrated_lufs: -22.5, true_peak_dbtp: -2.5, range_lu: 7.5 },
    peak_target_db: -1.5,
    boost_gain_db: 12.5,
};

/** A second payload, as the engine would return it for a different preset. */
export const effectiveNormalizeLoudStub: EffectiveNormalize = {
    preset: "loud",
    targets: { integrated_lufs: -10.5, true_peak_dbtp: -0.5, range_lu: 10.5 },
    peak_target_db: -1.5,
    boost_gain_db: 12.5,
};

/**
 * A stub preset catalogue (`loudnorm_presets` payload). Integrated-loudness
 * values are distinct from the real targets so a picker label can only match
 * if it is rendered from this payload.
 */
export const loudnormPresetsStub: LoudnormPresetInfo[] = [
    { preset: "streaming", targets: { integrated_lufs: -13, true_peak_dbtp: -0.5, range_lu: 10 } },
    { preset: "broadcast", targets: { integrated_lufs: -22, true_peak_dbtp: -1.5, range_lu: 6 } },
    { preset: "loud", targets: { integrated_lufs: -9, true_peak_dbtp: -0.5, range_lu: 10 } },
];
