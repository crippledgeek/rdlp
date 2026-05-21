// TanStack Query options for video codec availability.

import { queryOptions, skipToken } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { VideoCodecInfo } from "../types";

/**
 * Fetch available video codecs and their encoders.
 * Pass `ready: true` to enable the query; `false` gates via `skipToken`
 * so the queryFn identity changes and React 19 / TanStack v5 reliably
 * fires on the first true -> false -> true transition.
 */
export function codecsQueryOptions(ready: boolean) {
    return queryOptions({
        queryKey: queryKeys.codecs(),
        queryFn: ready
            ? () => invokeTyped<VideoCodecInfo[]>("available_codecs")
            : skipToken,
        staleTime: Infinity, // Encoder availability doesn't change at runtime
    });
}
