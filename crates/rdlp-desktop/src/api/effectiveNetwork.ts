// TanStack Query options for the engine's network/download payload: the
// RESOLVED values an empty field inherits, the BUILT-IN defaults, and the
// owning ranges — one command, one round trip (#611).

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { NetworkDefaults } from "../types";

/**
 * Fetch `NetworkDefaults` from the `network_defaults` command.
 *
 * `staleTime: Infinity`: the base configuration (`config.toml`) is loaded once
 * per process and the built-in defaults and ranges are compile-time constants,
 * so nothing in the payload can change while the app runs.
 */
export function networkDefaultsQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.networkDefaults(),
        queryFn: () => invokeTyped<NetworkDefaults>("network_defaults"),
        staleTime: Infinity,
    });
}
