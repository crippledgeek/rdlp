// Centralized query key factory.
//
// Every TanStack Query call MUST reference keys from here -- never
// inline key arrays in components. This prevents key drift and makes
// invalidation predictable.

import type { SearchFilter } from "../types";

export const queryKeys = {
    providers: () => ["providers"] as const,
    filters: (site: string) => ["filters", site] as const,
    search: (query: string, site: string, filters: SearchFilter[]) =>
        ["search", query, site, filters] as const,
    downloads: {
        list: () => ["downloads"] as const,
    },
    formats: (url: string) => ["formats", url] as const,
    settings: () => ["settings"] as const,
};
