// SearchResults: infinite scroll search results with TanStack Table sorting.
// Columns: Thumbnail, Title (sortable), Duration (sortable), Views (sortable), Date (sortable).
//
// Lazy enrichment: rows that scroll into view (or within 200 px of view)
// fire a `enrich_search_result` Tauri call to fill the uploader/etc. fields
// the search-page card markup couldn't surface. Cached per-row at the
// TanStack Query layer for an hour.

import { useEffect, useMemo, useRef, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useStore } from "@tanstack/react-store";
import {
    flexRender,
    getCoreRowModel,
    getSortedRowModel,
    useReactTable,
    type Row,
} from "@tanstack/react-table";
import type { SortingState } from "@tanstack/react-table";
import { searchStore } from "@/stores/searchStore";
import { enrichSearchResultQueryOptions, searchInfiniteQueryOptions } from "@/api/search";
import { setAnalyzeUrl } from "@/stores/uiStore";
import { Thumbnail } from "@/components/Thumbnail";
import { SearchFilterBar } from "./SearchFilterBar";
import { searchResultColumns } from "./searchResultColumns";
import { Skeleton } from "@/components/ui/skeleton";
import { Search, ArrowUpDown, ArrowUp, ArrowDown } from "lucide-react";
import type { SearchResultPreview } from "@/types";

/**
 * Per-row lazy enrichment: when the row scrolls into view (with a
 * 200 px pre-roll), kicks off `enrich_search_result` to backfill the
 * uploader / actors / tags from the underlying video page. The enriched
 * value, when present, overrides the original cell render for the
 * `uploader` column.
 */
function SearchResultRow({
    row,
    site,
}: {
    row: Row<SearchResultPreview>;
    site: string;
}) {
    const rowRef = useRef<HTMLTableRowElement>(null);
    const [isVisible, setIsVisible] = useState(false);

    useEffect(() => {
        const node = rowRef.current;
        if (!node) return;
        const observer = new IntersectionObserver(
            (entries) => {
                if (entries[0]?.isIntersecting) {
                    setIsVisible(true);
                }
            },
            { rootMargin: "200px" },
        );
        observer.observe(node);
        return () => observer.disconnect();
    }, []);

    const needsEnrichment = row.original.uploader == null;
    const enrichReady = isVisible && needsEnrichment;
    const { data: enriched, isFetching: isEnriching } = useQuery(
        enrichSearchResultQueryOptions(site, row.original, enrichReady),
    );

    const merged: SearchResultPreview = enriched ?? row.original;

    return (
        <tr
            ref={rowRef}
            className="border-b border-[#1a1a2e] hover:bg-[var(--surface-elevated)] cursor-pointer transition-colors"
            onClick={() => setAnalyzeUrl(row.original.video_url)}
        >
            {/* Thumbnail */}
            <td className="min-w-[72px] px-2 py-1.5">
                <div className="w-16 h-9 rounded-[3px] overflow-hidden bg-[var(--surface-elevated)] relative">
                    <Thumbnail
                        src={row.original.thumbnail_url}
                        alt=""
                        className="w-full h-full object-cover"
                    />
                    {row.original.duration != null && (
                        <span className="absolute bottom-0.5 right-0.5 bg-black/80 text-white text-[9px] px-1 rounded-[2px] font-mono">
                            {Math.floor(row.original.duration / 60)}:{String(row.original.duration % 60).padStart(2, "0")}
                        </span>
                    )}
                </div>
            </td>
            {/* Data columns — uploader cell uses the enriched value when present.
                While enrichment is in flight (isFetching=true and the row was
                missing an uploader to begin with), the cell pulses so the
                lazy-load behaviour is visible to the user. */}
            {row.getVisibleCells().map((cell) => {
                if (cell.column.id === "uploader") {
                    if (merged.uploader != null) {
                        return (
                            <td
                                key={cell.id}
                                className="px-2 py-1.5 text-[12px] text-[#aaaaaa] truncate"
                            >
                                {merged.uploader}
                            </td>
                        );
                    }
                    if (isEnriching) {
                        return (
                            <td
                                key={cell.id}
                                className="px-2 py-1.5"
                            >
                                <span className="inline-block h-3 w-20 rounded-[3px] bg-[var(--surface-elevated)] animate-pulse" />
                            </td>
                        );
                    }
                    // Enrichment resolved with no uploader — the video page
                    // genuinely has no studio / pornstar / profile link.
                    // Render a dimmer "—" so the user can distinguish
                    // "we tried and the page has nothing" (this state) from
                    // "still loading" (the pulse above) and from "we never
                    // attempted" (default — when isVisible was never true).
                    if (enriched && merged.uploader == null) {
                        return (
                            <td
                                key={cell.id}
                                className="px-2 py-1.5 text-[12px] text-[#555555] truncate italic"
                                title="Video page has no studio or creator attribution"
                            >
                                —
                            </td>
                        );
                    }
                    // Fall through to the column-defined cell ("—") — the
                    // initial state before IntersectionObserver fires.
                }
                return (
                    <td
                        key={cell.id}
                        className="px-2 py-1.5 text-[12px] text-[#aaaaaa] truncate"
                        style={{ width: cell.column.getSize() }}
                    >
                        {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </td>
                );
            })}
        </tr>
    );
}

interface SearchResultsProps {
    query: string;
    site: string;
}

export function SearchResults({ query, site }: SearchResultsProps) {
    const filters = useStore(searchStore, (s) => s.filters);
    const [sorting, setSorting] = useState<SortingState>([]);

    // queryKey is derived from (query, site, filters); TanStack Query
    // refetches automatically when any of them changes. `enabled` in the
    // options gates on query non-empty. No manual refetch Effect — `refetch`
    // is still exposed for the Retry button in the error state.
    const {
        data,
        isLoading,
        isError,
        isFetchingNextPage,
        fetchNextPage,
        hasNextPage,
        refetch,
    } = useInfiniteQuery(searchInfiniteQueryOptions(query, site, filters));

    const allResults = useMemo(
        () => data?.pages.flatMap((p) => p.results) ?? [],
        [data],
    );
    const totalEstimate = data?.pages[0]?.total_estimate;

    // Hide columns whose underlying field is empty for every loaded row, so
    // sites that don't surface a metadata axis (e.g. SpankBang search has no
    // date) don't show a permanently-blank column. Recomputed only when the
    // result set changes; column ids match the accessor keys / explicit ids
    // declared in `searchResultColumns`.
    const columnVisibility = useMemo<Record<string, boolean>>(() => {
        if (allResults.length === 0) {
            return { uploader: true, duration: true, views: true, age: true };
        }
        return {
            uploader: allResults.some((r) => r.uploader != null),
            duration: allResults.some((r) => r.duration != null),
            views: allResults.some((r) => r.view_count != null),
            age: allResults.some((r) => r.upload_date != null),
        };
    }, [allResults]);

    const table = useReactTable({
        data: allResults,
        columns: searchResultColumns,
        state: { sorting, columnVisibility },
        onSortingChange: setSorting,
        getCoreRowModel: getCoreRowModel(),
        getSortedRowModel: getSortedRowModel(),
    });

    // Compute percentage widths from visible columns' TanStack `size` totals,
    // plus a virtual size for the always-present thumbnail column. Per
    // CSS 2.2 §17.5.2.1, percentage widths on <col> under
    // table-layout: fixed are interpreted relative to the table's outer
    // width, so columns scale to fill 100% while preserving the relative
    // proportions declared in searchResultColumns.ts. The `|| 1` guard
    // handles the zero-column edge case (e.g. before any data has loaded).
    const THUMBNAIL_VIRTUAL_SIZE = 72;
    const visibleHeaders = table.getHeaderGroups()[0]?.headers ?? [];
    const dataColumnsSize = visibleHeaders.reduce(
        (sum, h) => sum + h.getSize(),
        0,
    );
    const totalSize = THUMBNAIL_VIRTUAL_SIZE + dataColumnsSize || 1;

    const filterBar = <SearchFilterBar />;

    if (isLoading) {
        return (
            <div className="flex flex-col h-full">
                {filterBar}
                <div className="flex flex-col gap-2 p-3">
                    {Array.from({ length: 6 }).map((_, i) => (
                        <Skeleton key={i} className="h-12 w-full rounded-[6px]" />
                    ))}
                </div>
            </div>
        );
    }

    if (isError) {
        return (
            <div className="flex flex-col h-full">
                {filterBar}
                <div className="flex flex-col items-center justify-center flex-1 gap-3 p-8">
                    <p className="text-[13px] text-[#e85858]">Search failed</p>
                    <button
                        onClick={() => void refetch()}
                        className="px-3 py-1 rounded-[4px] bg-[#4a9eff] text-white text-[12px] font-medium hover:bg-[#3a8ef0] transition-colors"
                    >
                        Retry
                    </button>
                </div>
            </div>
        );
    }

    if (allResults.length === 0) {
        return (
            <div className="flex flex-col h-full">
                {filterBar}
                <div className="flex flex-col items-center justify-center flex-1 gap-3 p-8">
                    <Search className="w-8 h-8 text-[#444444]" />
                    <p className="text-[13px] text-[#666666]">
                        {query.trim() ? "No results found" : "Type to search"}
                    </p>
                </div>
            </div>
        );
    }

    return (
        <div className="flex flex-col h-full">
            {filterBar}

            {/* Results count */}
            <div className="flex items-center gap-2 px-3 py-1.5 border-b border-[#1a1a2e] shrink-0">
                <span className="text-[11px] text-[#666666]">
                    {allResults.length}
                    {totalEstimate != null ? ` of ~${totalEstimate}` : ""}
                    {" "}results
                </span>
            </div>

            {/* Table */}
            <div className="flex-1 overflow-y-auto overflow-x-auto">
                <table
                    className="w-full min-w-[652px]"
                    style={{ tableLayout: "fixed" }}
                >
                    <colgroup>
                        <col
                            style={{
                                width: `${(THUMBNAIL_VIRTUAL_SIZE / totalSize) * 100}%`,
                            }}
                        />
                        {visibleHeaders.map((h) => (
                            <col
                                key={h.id}
                                style={{
                                    width: `${(h.getSize() / totalSize) * 100}%`,
                                }}
                            />
                        ))}
                    </colgroup>
                    <thead className="sticky top-0 bg-[var(--surface-deepest)] z-10">
                        {table.getHeaderGroups().map((headerGroup) => (
                            <tr key={headerGroup.id}>
                                {/* Thumbnail column header (non-sortable) */}
                                <th className="min-w-[72px] px-2 py-1.5" />
                                {headerGroup.headers.map((header) => {
                                    const sorted = header.column.getIsSorted();
                                    return (
                                        <th
                                            key={header.id}
                                            className="text-left px-2 py-1.5 text-[10px] uppercase tracking-[0.5px] text-[#555555] font-normal select-none cursor-pointer hover:text-[#888888] transition-colors"
                                            onClick={header.column.getToggleSortingHandler()}
                                        >
                                            <span className="flex items-center gap-1">
                                                {flexRender(header.column.columnDef.header, header.getContext())}
                                                {sorted === "asc" ? (
                                                    <ArrowUp className="w-3 h-3 text-[#4a9eff]" />
                                                ) : sorted === "desc" ? (
                                                    <ArrowDown className="w-3 h-3 text-[#4a9eff]" />
                                                ) : (
                                                    <ArrowUpDown className="w-3 h-3 opacity-30" />
                                                )}
                                            </span>
                                        </th>
                                    );
                                })}
                            </tr>
                        ))}
                    </thead>
                    <tbody>
                        {table.getRowModel().rows.map((row) => (
                            <SearchResultRow key={row.id} row={row} site={site} />
                        ))}
                    </tbody>
                </table>

                {/* Load more */}
                {hasNextPage && (
                    <button
                        onClick={() => void fetchNextPage()}
                        disabled={isFetchingNextPage}
                        className="w-full py-2 text-[12px] text-[#4a9eff] hover:text-[#3a8ef0] transition-colors disabled:opacity-50 mt-1"
                    >
                        {isFetchingNextPage ? "Loading…" : "Load more"}
                    </button>
                )}

                {!hasNextPage && allResults.length > 0 && (
                    <p className="text-center text-[11px] text-[#444444] py-2 mt-1">
                        All results loaded
                    </p>
                )}
            </div>
        </div>
    );
}
