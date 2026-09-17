// TanStack Query options for the engine's normalization values, resolved for
// a given preset: what an empty (inherit) target field actually runs with.

import { keepPreviousData, queryOptions, skipToken } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { LoudnormPreset } from "../types";

/**
 * Fetch the normalization values the engine runs with for `preset`
 * (`null` = inherit the base configuration's preset).
 *
 * `undefined` means the draft does not exist yet (the settings query has not
 * resolved), and the query is gated with `skipToken`: fetching for `null`
 * first and refetching once the stored preset arrives is a wasted round trip
 * and a wrong payload on screen for one render. Callers pass
 * `draft?.loudnorm_preset` and get exactly that distinction.
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
export function effectiveNormalizeQueryOptions(preset: LoudnormPreset | null | undefined) {
    return queryOptions({
        queryKey: queryKeys.effectiveNormalize(preset ?? null),
        queryFn:
            preset === undefined
                ? skipToken
                : () => invokeTyped("effective_normalize", { preset }),
        staleTime: Infinity,
        placeholderData: keepPreviousData,
    });
}

/**
 * Fetch every loudnorm preset with its targets (`LoudnormPreset::describe_all`),
 * for the preset picker's per-item labels. Compile-time constants on the Rust
 * side: never stale.
 */
export function loudnormPresetsQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.loudnormPresets(),
        queryFn: () => invokeTyped("loudnorm_presets"),
        staleTime: Infinity,
    });
}
