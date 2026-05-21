// TanStack Query options and mutations for settings-related Rust commands.

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import { queryClient } from "../query/queryClient";
import type { AppSettings } from "../types";

/** Fetch app settings. Stale for 5 minutes — rarely changes externally. */
export function settingsQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.settings(),
        queryFn: () => invokeTyped<AppSettings>("settings"),
        staleTime: 5 * 60_000,
    });
}

/** Update settings on the backend and optimistically update the cache. */
export async function updateSettings(settings: AppSettings): Promise<void> {
    await invokeTyped<void>("update_settings", { settings });
    queryClient.setQueryData(queryKeys.settings(), settings);
}

/** Open a native directory picker dialog. */
export async function pickDirectory(): Promise<string | null> {
    return invokeTyped<string | null>("pick_directory");
}
