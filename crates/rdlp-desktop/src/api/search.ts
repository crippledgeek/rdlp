// TanStack Query options for search-related Rust commands.

import { queryOptions, infiniteQueryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type {
    SearchFilter,
    SearchFilterDescriptor,
    SearchPageResponse,
    SearchSiteInfo,
} from "../types";

/** Fetch all search-capable sites. Runs once on app mount. */
export function providersQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.providers(),
        queryFn: () => invokeTyped<SearchSiteInfo[]>("get_search_providers"),
        staleTime: Infinity, // Providers don't change at runtime
    });
}

/** Fetch filter descriptors for a specific site. Re-fetches when site changes. */
export function filtersQueryOptions(site: string) {
    return queryOptions({
        queryKey: queryKeys.filters(site),
        queryFn: () =>
            invokeTyped<SearchFilterDescriptor[]>("get_search_filters", { site }),
        enabled: site !== "",
    });
}

/**
 * Infinite search query. `enabled: false` by default — must be triggered via `refetch()`.
 * The key includes query/site/filters so each unique combination gets its own cache entry.
 * pageParam is managed internally by useInfiniteQuery.
 */
export function searchInfiniteQueryOptions(
    query: string,
    site: string,
    filters: SearchFilter[],
) {
    return infiniteQueryOptions({
        queryKey: queryKeys.search.params(query, site, filters),
        queryFn: ({ pageParam }) =>
            invokeTyped<SearchPageResponse>("search_content", { query, site, filters, page: pageParam }),
        initialPageParam: 1,
        getNextPageParam: (lastPage) =>
            lastPage.has_more && lastPage.results.length > 0 ? lastPage.page + 1 : undefined,
        enabled: false,
        maxPages: 5,
    });
}
