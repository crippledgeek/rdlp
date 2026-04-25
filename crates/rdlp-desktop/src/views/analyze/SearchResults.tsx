// SearchResults: infinite scroll search results with TanStack Table sorting.
// Columns: Thumbnail, Title (sortable), Duration (sortable), Views (sortable), Date (sortable).

import { useMemo } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useStore } from "@tanstack/react-store";
import {
    flexRender,
    getCoreRowModel,
    getSortedRowModel,
    useReactTable,
} from "@tanstack/react-table";
import type { SortingState } from "@tanstack/react-table";
import { useState } from "react";
import { searchStore } from "@/stores/searchStore";
import { searchInfiniteQueryOptions } from "@/api/search";
import { setAnalyzeUrl } from "@/stores/uiStore";
import { Thumbnail } from "@/components/Thumbnail";
import { SearchFilterBar } from "./SearchFilterBar";
import { searchResultColumns } from "./searchResultColumns";
import { Skeleton } from "@/components/ui/skeleton";
import { Search, ArrowUpDown, ArrowUp, ArrowDown } from "lucide-react";

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
                <table className="w-full min-w-[652px]" style={{ tableLayout: "fixed" }}>
                    <thead className="sticky top-0 bg-[var(--surface-deepest)] z-10">
                        {table.getHeaderGroups().map((headerGroup) => (
                            <tr key={headerGroup.id}>
                                {/* Thumbnail column header (non-sortable) */}
                                <th className="w-[72px] px-2 py-1.5" />
                                {headerGroup.headers.map((header) => {
                                    const sorted = header.column.getIsSorted();
                                    return (
                                        <th
                                            key={header.id}
                                            className="text-left px-2 py-1.5 text-[10px] uppercase tracking-[0.5px] text-[#555555] font-normal select-none cursor-pointer hover:text-[#888888] transition-colors"
                                            style={{ width: header.getSize() }}
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
                            <tr
                                key={row.id}
                                className="border-b border-[#1a1a2e] hover:bg-[var(--surface-elevated)] cursor-pointer transition-colors"
                                onClick={() => setAnalyzeUrl(row.original.video_url)}
                            >
                                {/* Thumbnail */}
                                <td className="w-[72px] px-2 py-1.5">
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
                                {/* Data columns */}
                                {row.getVisibleCells().map((cell) => (
                                    <td
                                        key={cell.id}
                                        className="px-2 py-1.5 text-[12px] text-[#aaaaaa] truncate"
                                        style={{ width: cell.column.getSize() }}
                                    >
                                        {flexRender(cell.column.columnDef.cell, cell.getContext())}
                                    </td>
                                ))}
                            </tr>
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
