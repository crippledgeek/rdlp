// TanStack Query options for search-related Rust commands.

import { queryOptions, infiniteQueryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import type {
    SearchFilter,
    SearchFilterDescriptor,
    SearchPageResponse,
    SearchResultPreview,
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
 * Lazily enrich one search-result row with metadata the search-page card
 * markup did not surface (typically the uploader display name on tag-only
 * cards, plus actors / tags that only appear on the video page).
 *
 * Cached per `(site, video_url)` so a row that scrolls in/out/in only
 * triggers one upstream fetch. `staleTime` is 1 hour — preview metadata
 * for a given video URL is effectively immutable.
 *
 * `enabled` is gated by both the row being on-screen (caller passes the
 * IntersectionObserver result) and the preview already being incomplete.
 */
export function enrichSearchResultQueryOptions(
    site: string,
    preview: SearchResultPreview,
    enabled: boolean,
) {
    return queryOptions({
        queryKey: queryKeys.search.enrichRow(site, preview.video_url),
        queryFn: () =>
            invokeTyped<SearchResultPreview>("enrich_search_result", { site, preview }),
        enabled: enabled && site !== "" && preview.video_url !== "",
        staleTime: 60 * 60 * 1000,
        gcTime: 60 * 60 * 1000,
        retry: 0,
    });
}

/**
 * Infinite search query. Auto-fetches when `query` is non-empty; the key
 * includes query/site/filters so each unique combination gets its own cache
 * entry and TanStack Query handles refetching automatically when any of them
 * changes. No manual `refetch()` Effect needed in consumers.
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
        enabled: query.trim().length > 0,
        maxPages: 5,
    });
}
