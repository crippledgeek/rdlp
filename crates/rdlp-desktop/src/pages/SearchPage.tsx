import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { CommandBar } from "../components/CommandBar";
import { FilterBar } from "../components/FilterBar";
import { ResultsList } from "../components/ResultsList";
import { ResultCard } from "../components/ResultCard";
import { SearchIdleState } from "../components/SearchIdleState";
import { BatchActionBar } from "../components/BatchActionBar";
import { FormatDialog } from "../components/FormatDialog";
import {
    searchParamsAtom,
    setSearchParam,
} from "../stores/searchParamsStore";
import {
    providersQueryOptions,
    searchQueryOptions,
} from "../api/search";
import { settingsQueryOptions } from "../api/settings";
import {
    startDownload as apiStartDownload,
    buildDefaultOptions,
} from "../api/downloads";
import { useSearchHistory } from "../hooks/useSearchHistory";
import { useOnlineStatus } from "../hooks/useOnlineStatus";
import { useKeyboardNavigation } from "../hooks/useKeyboardNavigation";
import type { DownloadOptions, ViewMode } from "../types";

interface SearchPageProps {
    activeTab: string;
    viewMode: ViewMode;
}

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
    } = useQuery(searchQueryOptions(query, site, filters));
    const { data: settings = null } = useQuery(settingsQueryOptions());

    // Derive results and status from query state
    const results = searchData?.results ?? [];
    const status: "idle" | "loading" | "results" | "empty" | "error" = isFetching
        ? "loading"
        : isError
            ? "error"
            : isSuccess && results.length === 0
                ? "empty"
                : isSuccess
                    ? "results"
                    : "idle";

    const error = isError
        ? (queryError instanceof Error ? queryError.message : String(queryError))
        : null;

    const { addEntry } = useSearchHistory();
    const isOnline = useOnlineStatus();

    const [formatDialogUrl, setFormatDialogUrl] = useState<string | null>(null);
    const inputRef = useRef<HTMLInputElement>(null);

    // Record search history when results arrive
    useEffect(() => {
        if (status === "results" && query.trim() !== "" && site !== "") {
            const displayName =
                providers.find((p) => p.name === site)?.display_name ?? site;
            addEntry({ query, site, siteDisplayName: displayName, filters: [] });
        }
    }, [status]); // eslint-disable-line react-hooks/exhaustive-deps

    const handleDownload = useCallback(
        (url: string) => {
            const opts = buildDefaultOptions(settings);
            void apiStartDownload(url, opts);
        },
        [settings],
    );

    const handleDownloadWithOptions = useCallback(
        (url: string, options: Partial<DownloadOptions>) => {
            const defaults = buildDefaultOptions(settings);
            void apiStartDownload(url, { ...defaults, ...options });
        },
        [settings],
    );

    const handleOpenFormatDialog = useCallback(
        (url: string) => { setFormatDialogUrl(url); },
        [],
    );

    const handleFormatDialogConfirm = useCallback(
        (options: DownloadOptions) => {
            if (formatDialogUrl) {
                void apiStartDownload(formatDialogUrl, options);
            }
            setFormatDialogUrl(null);
        },
        [formatDialogUrl],
    );

    // Keyboard nav callbacks (by index)
    const handleDownloadByIndex = useCallback(
        (index: number) => {
            if (results[index]) handleDownload(results[index].video_url);
        },
        [results, handleDownload],
    );

    const handleFormatByIndex = useCallback(
        (index: number) => {
            if (results[index]) handleOpenFormatDialog(results[index].video_url);
        },
        [results, handleOpenFormatDialog],
    );

    const handleOpenInBrowser = useCallback(
        (index: number) => {
            if (results[index]) {
                window.open(results[index].video_url, "_blank");
            }
        },
        [results],
    );

    const {
        focusIndex,
        selectedIndices,
        toggleSelect,
        clearSelection,
    } = useKeyboardNavigation({
        resultCount: results.length,
        enabled: status === "results" && activeTab === "search",
        onDownload: handleDownloadByIndex,
        onOpenFormatDialog: handleFormatByIndex,
        onOpenInBrowser: handleOpenInBrowser,
    });

    const handleBatchDownload = useCallback(() => {
        for (const idx of selectedIndices) {
            if (results[idx]) handleDownload(results[idx].video_url);
        }
        clearSelection();
    }, [selectedIndices, results, handleDownload, clearSelection]);

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
            setTimeout(() => { void refetch(); }, 0);
        },
        [refetch],
    );

    const handleQuickSearch = useCallback(
        (q: string) => {
            setSearchParam("query", q);
            // Trigger search after atom update propagates
            setTimeout(() => { void refetch(); }, 0);
        },
        [refetch],
    );

    /** Reset filters to descriptor defaults then re-search. */
    const handleResetFiltersAndSearch = useCallback(() => {
        // FilterBar owns the filter descriptors and resets via its own
        // resetFiltersToDefaults. Here we just clear hasUserFilters and
        // set filters to empty (the FilterBar effect will re-apply
        // defaults from filter descriptors).
        searchParamsAtom.setState((prev) => ({
            ...prev,
            filters: [],
            hasUserFilters: false,
        }));
        setTimeout(() => { void refetch(); }, 0);
    }, [refetch]);

    return (
        <div className="search-page">
            <div className="search-command-center">
                <CommandBar inputRef={inputRef} activeTab={activeTab} />
                <FilterBar />
            </div>

            {/* Loading bar */}
            {status === "loading" && (
                <div className="search-loading-bar">
                    <div className="search-loading-bar-fill" />
                </div>
            )}

            {/* Offline banner */}
            {!isOnline && (
                <div className="search-banner search-banner-offline">
                    <span className="search-banner-dot" />
                    No internet connection
                    <button onClick={() => void refetch()}>Retry</button>
                </div>
            )}

            {/* Error banner */}
            {status === "error" && (
                <div className="search-banner search-banner-error">
                    {error ?? "An unknown error occurred."}
                    <button onClick={() => void refetch()}>Retry</button>
                </div>
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
                <div className="search-no-results">
                    <p>
                        No results for &ldquo;{query}&rdquo;
                        {site && ` on ${providers.find((p) => p.name === site)?.display_name ?? site}`}
                    </p>
                    <ul>
                        <li>Try different keywords</li>
                        {hasUserFilters && (
                            <li>
                                Remove active filters{" "}
                                <button
                                    className="search-link-btn"
                                    onClick={handleResetFiltersAndSearch}
                                >
                                    Clear filters
                                </button>
                            </li>
                        )}
                        <li>Search on a different site</li>
                    </ul>
                </div>
            )}

            {/* Results */}
            {status === "results" && viewMode === "list" && (
                <ResultsList
                    results={results}
                    focusIndex={focusIndex}
                    selectedIndices={selectedIndices}
                    onDownload={handleDownload}
                    onDownloadWithOptions={handleDownloadWithOptions}
                    onOpenFormatDialog={handleOpenFormatDialog}
                    onToggleSelect={toggleSelect}
                />
            )}

            {status === "results" && viewMode === "grid" && (
                <div className="results-grid results-grid-dense">
                    {results.map((result, idx) => (
                        <ResultCard
                            key={`${idx}-${result.video_url}`}
                            result={result}
                            onDownload={handleDownload}
                            onDownloadWithOptions={handleDownloadWithOptions}
                            onOpenFormatDialog={handleOpenFormatDialog}
                        />
                    ))}
                </div>
            )}

            {/* Batch action bar */}
            {selectedIndices.size > 0 && (
                <BatchActionBar
                    count={selectedIndices.size}
                    onDownloadAll={handleBatchDownload}
                    onClearSelection={clearSelection}
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
