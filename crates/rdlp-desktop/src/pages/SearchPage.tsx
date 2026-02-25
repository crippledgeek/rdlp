import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "@tanstack/react-store";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import {
    useReactTable,
    getCoreRowModel,
    getSortedRowModel,
} from "@tanstack/react-table";
import type { ColumnResizeMode, RowSelectionState, SortingState, Table } from "@tanstack/react-table";
import { WifiOff, AlertCircle, Loader2 } from "lucide-react";
import { CommandBar } from "../components/CommandBar";
import { FilterBar } from "../components/FilterBar";
import { ResultsTableSection } from "../components/ResultsTableSection";
import { ResultCard } from "../components/ResultCard";
import { SearchIdleState } from "../components/SearchIdleState";
import { BatchActionBar } from "../components/BatchActionBar";
import { FormatDialog } from "../components/FormatDialog";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
    searchParamsAtom,
    setSearchParam,
} from "../stores/searchParamsStore";
import {
    providersQueryOptions,
    searchInfiniteQueryOptions,
} from "../api/search";
import { settingsQueryOptions } from "../api/settings";
import {
    startDownload as apiStartDownload,
    buildDefaultOptions,
} from "../api/downloads";
import { useSearchHistory } from "../hooks/useSearchHistory";
import { useOnlineStatus } from "../hooks/useOnlineStatus";
import { useKeyboardNavigation } from "../hooks/useKeyboardNavigation";
import { useSearchColumns } from "../components/utils/searchColumns";
import type { DownloadOptions, SearchResultPreview, ViewMode } from "../types";

interface SearchPageProps {
    activeTab: string;
    viewMode: ViewMode;
}

const COLUMN_RESIZE_MODE: ColumnResizeMode = "onChange";

/** Main search page composing command bar, filters, results, and dialogs. */
export function SearchPage({ activeTab, viewMode }: SearchPageProps) {
    // --- TanStack Store: search form state ---
    const query = useStore(searchParamsAtom, (s) => s.query);
    const site = useStore(searchParamsAtom, (s) => s.site);
    const filters = useStore(searchParamsAtom, (s) => s.filters);
    const hasUserFilters = useStore(searchParamsAtom, (s) => s.hasUserFilters);

    // --- TanStack Query: server state ---
    const { data: providers = [] } = useQuery(providersQueryOptions());
    const {
        data: searchData,
        isFetching,
        isError,
        isSuccess,
        error: queryError,
        refetch,
        fetchNextPage,
        hasNextPage,
        isFetchingNextPage,
    } = useInfiniteQuery(searchInfiniteQueryOptions(query, site, filters));
    const { data: settings = null } = useQuery(settingsQueryOptions());

    // Derive results — memoized so TanStack Table doesn't reprocess rows on unrelated renders
    const results = useMemo(
        () => searchData?.pages.flatMap(p => p.results) ?? [],
        [searchData],
    );
    // Do not show the initial loading bar when only fetching additional pages
    const isInitialLoading = isFetching && !isFetchingNextPage;
    const status: "idle" | "loading" | "results" | "empty" | "error" = isInitialLoading
        ? "loading"
        : isError
            ? "error"
            : isSuccess && results.length === 0
                ? "empty"
                : isSuccess
                    ? "results"
                    : "idle";

    const error = isError
        ? (queryError instanceof Error
            ? queryError.message
            : (typeof queryError === "object" && queryError !== null && "message" in queryError)
                ? String((queryError as Record<string, unknown>).message)
                : String(queryError))
        : null;

    const { addEntry } = useSearchHistory();
    const isOnline = useOnlineStatus();

    const [formatDialogUrl, setFormatDialogUrl] = useState<string | null>(null);
    const inputRef = useRef<HTMLInputElement>(null);
    const tableRef = useRef<Table<SearchResultPreview> | null>(null);
    const allResultsLoadedRef = useRef<HTMLParagraphElement>(null);

    // --- TanStack Table state ---
    const [sorting, setSorting] = useState<SortingState>([]);
    const [rowSelection, setRowSelection] = useState<RowSelectionState>({});
    const [userColumnVisibility, setUserColumnVisibility] = useState<Record<string, boolean>>({});

    // Data-driven visibility: hide columns when no result has data for them
    const dataColumnVisibility = useMemo(() => {
        if (results.length === 0) return {} as Record<string, boolean>;
        return {
            age: results.some((r) => r.upload_date !== null),
        } as Record<string, boolean>;
    }, [results]);

    // Merge: data defaults overridden by explicit user toggles
    const columnVisibility = useMemo(
        () => ({ ...dataColumnVisibility, ...userColumnVisibility }),
        [dataColumnVisibility, userColumnVisibility],
    );

    // Reset table state when query/site/filters change (not on Load More page appends)
    useEffect(() => {
        setSorting([]);
        setRowSelection({});
        setUserColumnVisibility({});
    }, [query, site, filters]);

    // Focus "All results loaded" message when Load More button disappears (fix 8)
    useEffect(() => {
        if (!hasNextPage && !isFetchingNextPage && isSuccess && results.length > 0) {
            allResultsLoadedRef.current?.focus();
        }
    }, [hasNextPage, isFetchingNextPage, isSuccess, results.length]);

    // Record search history when results arrive
    useEffect(() => {
        const hasCategoryFilter = filters.some((f) => f.key === "category" && f.value !== "");
        if (status === "results" && (query.trim() !== "" || hasCategoryFilter) && site !== "") {
            const displayName =
                providers.find((p) => p.name === site)?.display_name ?? site;
            addEntry({ query, site, siteDisplayName: displayName, filters: [] });
        }
    }, [status]); // eslint-disable-line react-hooks/exhaustive-deps

    const handleSearch = useCallback(() => {
        refetch().catch((e) => console.error("Refetch failed:", e));
    }, [refetch]);

    const handleDownload = useCallback(
        (url: string, title?: string) => {
            const opts = buildDefaultOptions(settings);
            apiStartDownload(url, opts, title).catch((e) =>
                console.error("Download failed:", e),
            );
        },
        [settings],
    );

    const handleDownloadWithOptions = useCallback(
        (url: string, title: string, options: Partial<DownloadOptions>) => {
            const defaults = buildDefaultOptions(settings);
            apiStartDownload(url, { ...defaults, ...options }, title).catch((e) =>
                console.error("Download failed:", e),
            );
        },
        [settings],
    );

    const handleOpenFormatDialog = useCallback(
        (url: string) => { setFormatDialogUrl(url); },
        [],
    );

    const handleFormatDialogConfirm = useCallback(
        (options: DownloadOptions, title?: string) => {
            if (formatDialogUrl) {
                apiStartDownload(formatDialogUrl, options, title).catch((e) =>
                    console.error("Download failed:", e),
                );
            }
            setFormatDialogUrl(null);
        },
        [formatDialogUrl],
    );

    // Keyboard nav callbacks (by index, resolved through table rows via ref)
    const handleDownloadByIndex = useCallback(
        (index: number) => {
            const row = tableRef.current?.getRowModel().rows[index];
            if (row) handleDownload(row.original.video_url, row.original.title);
        },
        [handleDownload],
    );

    const handleFormatByIndex = useCallback(
        (index: number) => {
            const row = tableRef.current?.getRowModel().rows[index];
            if (row) handleOpenFormatDialog(row.original.video_url);
        },
        [handleOpenFormatDialog],
    );

    const handleOpenInBrowser = useCallback(
        (index: number) => {
            const row = tableRef.current?.getRowModel().rows[index];
            if (row) window.open(row.original.video_url, "_blank");
        },
        [],
    );

    // focusIndex is managed by keyboard nav; initialized to -1
    const [focusIndex, setFocusIndex] = useState(-1);

    const { columns, totalColumnSize } = useSearchColumns({
        onDownload: handleDownload,
        onDownloadWithOptions: handleDownloadWithOptions,
        onOpenFormatDialog: handleOpenFormatDialog,
        focusIndex,
    });

    const table = useReactTable({
        data: results,
        columns,
        state: {
            sorting,
            rowSelection,
            columnVisibility,
        },
        onSortingChange: setSorting,
        onRowSelectionChange: setRowSelection,
        onColumnVisibilityChange: setUserColumnVisibility,
        getCoreRowModel: getCoreRowModel(),
        getSortedRowModel: getSortedRowModel(),
        columnResizeMode: COLUMN_RESIZE_MODE,
        enableRowSelection: true,
    });

    // Keep ref current for index-based callbacks
    tableRef.current = table;

    useKeyboardNavigation({
        focusIndex,
        setFocusIndex,
        resultCount: results.length,
        enabled: status === "results" && activeTab === "search",
        table,
        onDownload: handleDownloadByIndex,
        onOpenFormatDialog: handleFormatByIndex,
        onOpenInBrowser: handleOpenInBrowser,
    });

    const selectedCount = Object.keys(rowSelection).length;

    const handleBatchDownload = useCallback(() => {
        const selectedRows = table.getSelectedRowModel().rows;
        for (const row of selectedRows) {
            handleDownload(row.original.video_url, row.original.title);
        }
        setRowSelection({});
    }, [table, handleDownload]);

    const handleClearSelection = useCallback(() => {
        setRowSelection({});
    }, []);

    const handleRestoreSearch = useCallback(
        (q: string, s: string, restoredFilters: Array<{ key: string; value: string }>) => {
            setSearchParam("query", q);
            setSearchParam("site", s);
            if (restoredFilters.length > 0) {
                searchParamsAtom.setState((prev) => ({
                    ...prev,
                    filters: restoredFilters,
                    hasUserFilters: true,
                }));
            }
            // Trigger search after atom updates propagate
            setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
        },
        [refetch],
    );

    const handleQuickSearch = useCallback(
        (q: string) => {
            setSearchParam("query", q);
            // Trigger search after atom update propagates
            setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
        },
        [refetch],
    );

    /** Reset filters to descriptor defaults then re-search. */
    const handleResetFiltersAndSearch = useCallback(() => {
        searchParamsAtom.setState((prev) => ({
            ...prev,
            filters: [],
            hasUserFilters: false,
        }));
        setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
    }, [refetch]);

    return (
        <div className="flex flex-col h-full">
            <div className="shrink-0 pb-2.5 border-b border-border mb-1.5">
                <CommandBar inputRef={inputRef} activeTab={activeTab} isFetching={isFetching} onSearch={handleSearch} />
                <FilterBar isFetching={isFetching} hasSearchData={!!searchData} onSearch={handleSearch} />
            </div>

            {/* Loading bar */}
            {status === "loading" && (
                <div className="h-0.5 bg-muted overflow-hidden shrink-0">
                    <div className="h-full w-2/5 bg-primary rounded-sm animate-[loading-sweep_1.5s_ease-in-out_infinite]" />
                </div>
            )}

            {/* Offline banner */}
            {!isOnline && (
                <Alert className="mb-1.5 bg-gray-500/[0.08] border-gray-500/15 text-muted-foreground">
                    <WifiOff className="h-4 w-4" />
                    <AlertDescription className="flex items-center gap-2.5">
                        No internet connection
                        <Button variant="outline" size="sm" onClick={() => { refetch().catch((e) => console.error("Refetch failed:", e)); }} className="ml-auto h-6 text-[11px]">Retry</Button>
                    </AlertDescription>
                </Alert>
            )}

            {/* Error banner */}
            {status === "error" && (
                <Alert variant="destructive" className="mb-1.5">
                    <AlertCircle className="h-4 w-4" />
                    <AlertDescription className="flex items-center gap-2.5">
                        {error ?? "An unknown error occurred."}
                        <Button variant="outline" size="sm" onClick={() => { refetch().catch((e) => console.error("Refetch failed:", e)); }} className="ml-auto h-6 text-[11px]">Retry</Button>
                    </AlertDescription>
                </Alert>
            )}

            {/* Idle state */}
            {status === "idle" && (
                <SearchIdleState
                    currentSite={site}
                    onRestoreSearch={handleRestoreSearch}
                    onQuickSearch={handleQuickSearch}
                />
            )}

            {/* No results */}
            {status === "empty" && (
                <div className="p-6 text-muted-foreground text-[13px]">
                    <p className="text-muted-foreground mb-2.5">
                        No results for &ldquo;{query}&rdquo;
                        {site && ` on ${providers.find((p) => p.name === site)?.display_name ?? site}`}
                    </p>
                    <ul className="list-disc pl-4">
                        <li className="mb-1">Try different keywords</li>
                        {hasUserFilters && (
                            <li className="mb-1">
                                Remove active filters{" "}
                                <button
                                    className="text-primary hover:text-primary/80 underline cursor-pointer bg-transparent border-none font-inherit text-inherit p-0"
                                    onClick={handleResetFiltersAndSearch}
                                >
                                    Clear filters
                                </button>
                            </li>
                        )}
                        <li className="mb-1">Search on a different site</li>
                    </ul>
                </div>
            )}

            {/* aria-live region: announces result count to screen readers when pages are appended */}
            <div className="sr-only" aria-live="polite" role="status">
                {results.length > 0 ? `${results.length} results loaded` : ""}
            </div>

            {/* Results: table view */}
            {status === "results" && viewMode === "list" && (
                <ResultsTableSection
                    table={table}
                    columns={columns}
                    totalColumnSize={totalColumnSize}
                    focusIndex={focusIndex}
                    resultCount={results.length}
                />
            )}

            {/* Results: grid view */}
            {status === "results" && viewMode === "grid" && (
                <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-3">
                    {results.map((result) => (
                        <ResultCard
                            key={result.video_url}
                            result={result}
                            onDownload={handleDownload}
                            onDownloadWithOptions={handleDownloadWithOptions}
                            onOpenFormatDialog={handleOpenFormatDialog}
                        />
                    ))}
                </div>
            )}

            {/* Load more */}
            {status === "results" && hasNextPage && (
                <div className="flex justify-center py-4">
                    <Button
                        variant="outline"
                        onClick={() => fetchNextPage()}
                        disabled={isFetchingNextPage}
                        aria-busy={isFetchingNextPage}
                        className="min-w-[200px] gap-1.5"
                    >
                        {isFetchingNextPage ? (
                            <>
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                Loading...
                            </>
                        ) : "Load more results"}
                    </Button>
                </div>
            )}

            {/* All results loaded */}
            {status === "results" && !hasNextPage && !isFetchingNextPage && (
                <p
                    ref={allResultsLoadedRef}
                    className="text-center text-muted-foreground text-xs py-3"
                    tabIndex={-1}
                >
                    All results loaded
                </p>
            )}

            {/* Batch action bar */}
            {selectedCount > 0 && (
                <BatchActionBar
                    count={selectedCount}
                    onDownloadAll={handleBatchDownload}
                    onClearSelection={handleClearSelection}
                />
            )}

            {/* Format dialog */}
            {formatDialogUrl && (
                <FormatDialog
                    url={formatDialogUrl}
                    onConfirm={handleFormatDialogConfirm}
                    onClose={() => setFormatDialogUrl(null)}
                />
            )}
        </div>
    );
}
