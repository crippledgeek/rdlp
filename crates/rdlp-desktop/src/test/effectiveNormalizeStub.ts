// A stub `EffectiveNormalize` payload for section tests.
//
// Every value is deliberately DISTINCT from the engine's real defaults
// (Streaming −14/−1/11, peak −1.0, boost 12.0) and from every other field, so
// a placeholder assertion can only pass if the section really derives it from
// this payload — a leftover literal, or a field wired to the wrong key, fails.

import type { EffectiveNormalize } from "@/types";

export const effectiveNormalizeStub: EffectiveNormalize = {
    preset: "broadcast",
    target_i: -22.5,
    target_tp: -2.5,
    target_lra: 7.5,
    peak_target_db: -1.5,
    boost_gain_db: 12.5,
};

/** A second payload, as the engine would return it for a different preset. */
export const effectiveNormalizeLoudStub: EffectiveNormalize = {
    preset: "loud",
    target_i: -10.5,
    target_tp: -0.5,
    target_lra: 10.5,
    peak_target_db: -1.5,
    boost_gain_db: 12.5,
};
