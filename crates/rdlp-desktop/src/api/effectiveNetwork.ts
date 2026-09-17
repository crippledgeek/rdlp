// TanStack Query options for the engine's resolved network/download settings.

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { EffectiveNetwork } from "../types";

/**
 * Fetch the values the engine runs with when a setting is left to inherit.
 *
 * `staleTime: Infinity`: the base configuration (`config.toml`) is loaded once
 * per process, so the resolved values cannot change while the app runs.
 */
export function effectiveNetworkQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.effectiveNetwork(),
        queryFn: () => invokeTyped<EffectiveNetwork>("effective_network"),
        staleTime: Infinity,
    });
}
