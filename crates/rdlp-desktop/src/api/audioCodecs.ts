// TanStack Query options for audio codec availability.

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { AudioCodecInfo } from "../types";

/**
 * Fetch available audio codecs, optionally filtered by container.
 * When container is null, returns all available audio codecs.
 * Runs once per (container) value — encoder availability doesn't change at runtime.
 */
export function audioCodecsQueryOptions(container: string | null) {
    return queryOptions({
        queryKey: queryKeys.audioCodecs.byContainer(container),
        queryFn: () =>
            invokeTyped<AudioCodecInfo[]>("get_available_audio_codecs", {
                container,
            }),
        staleTime: Infinity,
    });
}
