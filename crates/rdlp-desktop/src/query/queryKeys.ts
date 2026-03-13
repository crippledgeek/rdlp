// Centralized query key factory.
//
// Every TanStack Query call MUST reference keys from here -- never
// inline key arrays in components. This prevents key drift and makes
// invalidation predictable.
//
// Search keys are hierarchical for granular invalidation:
//   search.all()              → ["search"]           — invalidate everything
//   search.site("xhamster")   → ["search","xhamster"] — invalidate one site
//   search.params(q, s, f)    → ["search","xhamster","teen","sort=relevance"]
//
// Filters are serialized to a stable string so TanStack Query can
// compare keys with === instead of deep-comparing object arrays.

import type { SearchFilter } from "../types";

/** Deterministic string key for a filter set (sorted for stability). */
function serializeFilters(filters: SearchFilter[]): string {
    if (filters.length === 0) return "";
    return filters
        .map((f) => `${f.key}=${f.value}`)
        .sort()
        .join("&");
}

export const queryKeys = {
    providers: () => ["providers"] as const,
    filters: (site: string) => ["filters", site] as const,
    search: {
        all: () => ["search"] as const,
        site: (site: string) => ["search", site] as const,
        params: (query: string, site: string, filters: SearchFilter[]) =>
            ["search", site, query, serializeFilters(filters)] as const,
    },
    downloads: {
        list: () => ["downloads"] as const,
    },
    formats: (url: string) => ["formats", url] as const,
    settings: () => ["settings"] as const,
    thumbnail: {
        proxy: (url: string | null | undefined) => ["proxy-thumbnail", url] as const,
    },
};
