// TanStack Query options for video codec availability.

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { VideoCodecInfo } from "../types";

/** Fetch available video codecs and their encoders. Runs once per session. */
export function codecsQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.codecs(),
        queryFn: () => invokeTyped<VideoCodecInfo[]>("get_available_codecs"),
        staleTime: Infinity, // Encoder availability doesn't change at runtime
    });
}
