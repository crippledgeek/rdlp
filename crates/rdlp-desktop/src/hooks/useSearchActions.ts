import { useCallback, useRef, useState } from "react";
import type { Table } from "@tanstack/react-table";
import {
    searchParamsAtom,
    setSearchParam,
} from "../stores/searchParamsStore";
import {
    startDownload as apiStartDownload,
    buildDefaultOptions,
} from "../api/downloads";
import type { AppSettings, DownloadOptions, SearchResultPreview } from "../types";

interface UseSearchActionsParams {
    settings: AppSettings | null;
    refetch: () => Promise<unknown>;
}

/** Encapsulates download, format dialog, and search navigation actions for SearchPage. */
export function useSearchActions({ settings, refetch }: UseSearchActionsParams) {
    const tableRef = useRef<Table<SearchResultPreview> | null>(null);
    const [formatDialogUrl, setFormatDialogUrl] = useState<string | null>(null);

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

    const handleCloseFormatDialog = useCallback(() => {
        setFormatDialogUrl(null);
    }, []);

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
            setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
        },
        [refetch],
    );

    const handleQuickSearch = useCallback(
        (q: string) => {
            setSearchParam("query", q);
            setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
        },
        [refetch],
    );

    const handleResetFiltersAndSearch = useCallback(() => {
        searchParamsAtom.setState((prev) => ({
            ...prev,
            filters: [],
            hasUserFilters: false,
        }));
        setTimeout(() => { refetch().catch((e) => console.error("Refetch failed:", e)); }, 0);
    }, [refetch]);

    return {
        tableRef,
        formatDialogUrl,
        handleSearch,
        handleDownload,
        handleDownloadWithOptions,
        handleOpenFormatDialog,
        handleFormatDialogConfirm,
        handleCloseFormatDialog,
        handleDownloadByIndex,
        handleFormatByIndex,
        handleOpenInBrowser,
        handleRestoreSearch,
        handleQuickSearch,
        handleResetFiltersAndSearch,
    };
}
