// TanStack Query options for audio codec availability.

import { queryOptions, skipToken } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { AudioCodecInfo } from "../types";

/**
 * Fetch available audio codecs, optionally filtered by container.
 * When container is null, returns all available audio codecs.
 * Runs once per (container) value — encoder availability doesn't change at runtime.
 *
 * Pass `ready: true` to enable the query; `false` gates via `skipToken`
 * so the queryFn identity changes and React 19 / TanStack v5 reliably
 * fires on the first true -> false -> true transition.
 */
export function audioCodecsQueryOptions(container: string | null, ready: boolean) {
    return queryOptions({
        queryKey: queryKeys.audioCodecs.byContainer(container),
        queryFn: ready
            ? () =>
                  invokeTyped<AudioCodecInfo[]>("available_audio_codecs", {
                      container,
                  })
            : skipToken,
        staleTime: Infinity,
    });
}
