// TanStack Query options for format-related Rust commands.

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type { FormatListResponse } from "../types";

/** Fetch available formats for a URL. Used by FormatDialog. */
export function formatsQueryOptions(url: string | null) {
    return queryOptions({
        queryKey: queryKeys.formats(url ?? ""),
        queryFn: () => invokeTyped<FormatListResponse>("get_formats", { url: url! }),
        enabled: url !== null && url !== "",
    });
}
