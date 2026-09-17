// TanStack Query options for the engine's normalization values, resolved for
// a given preset: what an empty (inherit) target field actually runs with.

import { keepPreviousData, queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { EffectiveNormalize, LoudnormPreset } from "../types";

/**
 * Fetch the normalization values the engine runs with for `preset`
 * (`null` = inherit the base configuration's preset).
 *
 * Keyed by preset because the I/TP/LRA defaults are preset-dependent — one
 * cached payload cannot serve every preset, which is exactly how the
 * Streaming-only placeholders went wrong (#611).
 *
 * `staleTime: Infinity`: the base configuration is loaded once per process,
 * so a preset's resolved values cannot change while the app runs.
 * `placeholderData: keepPreviousData`: switching the preset in the draft
 * keeps the previous preset's payload on screen until the new one lands,
 * so the placeholders never blank mid-switch.
 */
export function effectiveNormalizeQueryOptions(preset: LoudnormPreset | null) {
    return queryOptions({
        queryKey: queryKeys.effectiveNormalize(preset),
        queryFn: () => invokeTyped<EffectiveNormalize>("effective_normalize", { preset }),
        staleTime: Infinity,
        placeholderData: keepPreviousData,
    });
}
