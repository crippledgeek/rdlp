// TanStack Query options for the engine's network/download values: the
// RESOLVED ones an empty field inherits, and the BUILT-IN defaults.

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

/**
 * Fetch the built-in defaults (`EffectiveNetwork::DEFAULT`), before any
 * `config.toml` layering. Used to seed a field when the inherited value cannot
 * express the user's intent (re-enabling idle eviction over an inherited `0`).
 * Compile-time constants on the Rust side: never stale.
 */
export function builtinNetworkDefaultsQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.builtinNetworkDefaults(),
        queryFn: () => invokeTyped<EffectiveNetwork>("builtin_network_defaults"),
        staleTime: Infinity,
    });
}
