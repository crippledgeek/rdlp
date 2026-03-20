// Custom hook encapsulating all SearchPage state, queries, and callbacks.

import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "@tanstack/react-store";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import {
    getCoreRowModel,
    getSortedRowModel,
} from "@tanstack/react-table";
import type { ColumnResizeMode, RowSelectionState, SortingState, Table } from "@tanstack/react-table";
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
import { useSearchHistory } from "./useSearchHistory";
import { useOnlineStatus } from "./useOnlineStatus";
import { useKeyboardNavigation } from "./useKeyboardNavigation";
import { useTable } from "./useTable";
import { useSearchColumns } from "../components/utils/searchColumns";
import type { DownloadOptions, SearchResultPreview } from "../types";

const COLUMN_RESIZE_MODE: ColumnResizeMode = "onChange";

export type SearchStatus = "idle" | "loading" | "results" | "empty" | "error";

interface UseSearchPageOptions {
    activeTab: string;
}

export function useSearchPage({ activeTab }: UseSearchPageOptions) {
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
    const status: SearchStatus = isInitialLoading
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
            views: results.some((r) => r.view_count !== null),
            age: results.some((r) => r.upload_date !== null),
        } as Record<string, boolean>;
    }, [results]);

    // Merge: data defaults overridden by explicit user toggles
    const columnVisibility = { ...dataColumnVisibility, ...userColumnVisibility };

    // Reset table state when query/site/filters change (not on Load More page appends)
    useEffect(() => {
        setSorting([]);
        setRowSelection({});
        setUserColumnVisibility({});
    }, [query, site, filters]);

    // Focus "All results loaded" message when Load More button disappears
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

    // --- Handlers ---

    const handleSearch = () => {
        refetch().catch((e) => console.error("Refetch failed:", e));
    };

    const handleDownload = (url: string, title?: string) => {
        const opts = buildDefaultOptions(settings);
        apiStartDownload(url, opts, title).catch((e) =>
            console.error("Download failed:", e),
        );
    };

    const handleDownloadWithOptions = (url: string, title: string, options: Partial<DownloadOptions>) => {
        const defaults = buildDefaultOptions(settings);
        apiStartDownload(url, { ...defaults, ...options }, title).catch((e) =>
            console.error("Download failed:", e),
        );
    };

    const handleOpenFormatDialog = (url: string) => {
        setFormatDialogUrl(url);
    };

    const handleFormatDialogConfirm = (options: DownloadOptions, title?: string) => {
        if (formatDialogUrl) {
            apiStartDownload(formatDialogUrl, options, title).catch((e) =>
                console.error("Download failed:", e),
            );
        }
        setFormatDialogUrl(null);
    };

    // Keyboard nav callbacks (by index, resolved through table rows via ref)
    const handleDownloadByIndex = (index: number) => {
        const row = tableRef.current?.getRowModel().rows[index];
        if (row) handleDownload(row.original.video_url, row.original.title);
    };

    const handleFormatByIndex = (index: number) => {
        const row = tableRef.current?.getRowModel().rows[index];
        if (row) handleOpenFormatDialog(row.original.video_url);
    };

    const handleOpenInBrowser = (index: number) => {
        const row = tableRef.current?.getRowModel().rows[index];
        if (row) window.open(row.original.video_url, "_blank");
    };

    // focusIndex is managed by keyboard nav; initialized to -1
    const [focusIndex, setFocusIndex] = useState(-1);

    const { columns, totalColumnSize } = useSearchColumns({
        onDownload: handleDownload,
        onDownloadWithOptions: handleDownloadWithOptions,
        onOpenFormatDialog: handleOpenFormatDialog,
        focusIndex,
    });

    const table = useTable({
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

    // Keep ref current for index-based callbacks (in effect, not render)
    useEffect(() => { tableRef.current = table; });

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

    const handleBatchDownload = () => {
        const selectedRows = table.getSelectedRowModel().rows;
        for (const row of selectedRows) {
            handleDownload(row.original.video_url, row.original.title);
        }
        setRowSelection({});
    };

    const handleClearSelection = () => {
        setRowSelection({});
    };

    const handleRestoreSearch = (q: string, s: string, restoredFilters: Array<{ key: string; value: string }>) => {
        setSearchParam("query", q);
        setSearchParam("site", s);
        if (restoredFilters.length > 0) {
            searchParamsAtom.setState((prev) => ({
                ...prev,
                filters: restoredFilters,
                hasUserFilters: true,
            }));
        }
        setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
    };

    const handleQuickSearch = (q: string) => {
        setSearchParam("query", q);
        setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
    };

    const handleResetFiltersAndSearch = () => {
        searchParamsAtom.setState((prev) => ({
            ...prev,
            filters: [],
            hasUserFilters: false,
        }));
        setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
    };

    return {
        // State
        query,
        site,
        hasUserFilters,
        status,
        error,
        results,
        isOnline,
        isFetching,
        isFetchingNextPage,
        hasNextPage,
        searchData,
        providers,
        formatDialogUrl,
        inputRef,
        allResultsLoadedRef,
        focusIndex,
        selectedCount,
        table,
        columns,
        totalColumnSize,
        sorting,

        // Actions
        handleSearch,
        handleDownload,
        handleDownloadWithOptions,
        handleOpenFormatDialog,
        handleFormatDialogConfirm,
        handleBatchDownload,
        handleClearSelection,
        handleRestoreSearch,
        handleQuickSearch,
        handleResetFiltersAndSearch,
        setFormatDialogUrl,
        refetch,
        fetchNextPage,
    };
}
