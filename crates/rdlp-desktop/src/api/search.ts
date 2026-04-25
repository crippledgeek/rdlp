// TanStack Query options for search-related Rust commands.

import { queryOptions, infiniteQueryOptions, skipToken } from "@tanstack/react-query";
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
    const ready = enabled && site !== "" && preview.video_url !== "";
    // TanStack Query v5 documented lazy-query pattern: switch the queryFn
    // between `skipToken` (disabled, type-safe) and a real function based on
    // the gating condition. Toggling `enabled: false → true` on a
    // queryOptions() factory has known propagation quirks under React 19;
    // skipToken changes the query identity itself which TanStack always
    // re-evaluates.
    // https://tanstack.com/query/v5/docs/react/guides/disabling-queries
    return queryOptions({
        queryKey: queryKeys.search.enrichRow(site, preview.video_url),
        queryFn: ready
            ? async () => {
                  // eslint-disable-next-line no-console
                  console.debug("[enrich] queryFn START:", preview.video_url);
                  try {
                      const result = await invokeTyped<SearchResultPreview>(
                          "enrich_search_result",
                          { site, preview },
                      );
                      // eslint-disable-next-line no-console
                      console.debug(
                          "[enrich] queryFn DONE:",
                          preview.video_url,
                          "→ uploader =",
                          result.uploader,
                      );
                      return result;
                  } catch (err) {
                      // eslint-disable-next-line no-console
                      console.error(
                          "[enrich] queryFn THREW:",
                          preview.video_url,
                          err,
                      );
                      throw err;
                  }
              }
            : skipToken,
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
