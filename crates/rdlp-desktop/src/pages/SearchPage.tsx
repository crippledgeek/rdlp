import { useCallback, useRef, useState } from "react";
import { CommandBar } from "../components/CommandBar";
import { FilterBar } from "../components/FilterBar";
import { ResultsList } from "../components/ResultsList";
import { ResultCard } from "../components/ResultCard";
import { SearchIdleState } from "../components/SearchIdleState";
import { BatchActionBar } from "../components/BatchActionBar";
import { FormatDialog } from "../components/FormatDialog";
import { useSearchStore, useQueueStore } from "../lib/store";
import { useOnlineStatus } from "../hooks/useOnlineStatus";
import { useKeyboardNavigation } from "../hooks/useKeyboardNavigation";
import type { DownloadOptions, ViewMode } from "../types";

interface SearchPageProps {
    activeTab: string;
    viewMode: ViewMode;
}

/** Main search page composing command bar, filters, results, and dialogs. */
export function SearchPage({ activeTab, viewMode }: SearchPageProps) {
    const status = useSearchStore((s) => s.status);
    const results = useSearchStore((s) => s.results);
    const error = useSearchStore((s) => s.error);
    const search = useSearchStore((s) => s.search);
    const query = useSearchStore((s) => s.query);
    const site = useSearchStore((s) => s.site);
    const setQuery = useSearchStore((s) => s.setQuery);
    const setSite = useSearchStore((s) => s.setSite);
    const setFilters = useSearchStore((s) => s.setFilters);
    const hasUserFilters = useSearchStore((s) => s.hasUserFilters);
    const resetFiltersToDefaults = useSearchStore((s) => s.resetFiltersToDefaults);
    const providers = useSearchStore((s) => s.providers);
    const startDownload = useQueueStore((s) => s.startDownload);

    const isOnline = useOnlineStatus();

    const [formatDialogUrl, setFormatDialogUrl] = useState<string | null>(null);
    const inputRef = useRef<HTMLInputElement>(null);

    const handleDownload = useCallback(
        (url: string) => { void startDownload(url); },
        [startDownload],
    );

    const handleDownloadWithOptions = useCallback(
        (url: string, options: Partial<DownloadOptions>) => {
            void startDownload(url, options);
        },
        [startDownload],
    );

    const handleOpenFormatDialog = useCallback(
        (url: string) => { setFormatDialogUrl(url); },
        [],
    );

    const handleFormatDialogConfirm = useCallback(
        (options: DownloadOptions) => {
            if (formatDialogUrl) {
                void startDownload(formatDialogUrl, options);
            }
            setFormatDialogUrl(null);
        },
        [formatDialogUrl, startDownload],
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
        (q: string, s: string, filters: Array<{ key: string; value: string }>) => {
            setQuery(q);
            setSite(s);
            if (filters.length > 0) setFilters(filters);
            // Trigger search after state updates
            setTimeout(() => { void search(); }, 0);
        },
        [setQuery, setSite, setFilters, search],
    );

    const handleQuickSearch = useCallback(
        (q: string) => {
            setQuery(q);
            setTimeout(() => { void search(); }, 0);
        },
        [setQuery, search],
    );

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
                    <button onClick={() => void search()}>Retry</button>
                </div>
            )}

            {/* Error banner */}
            {status === "error" && (
                <div className="search-banner search-banner-error">
                    {error ?? "An unknown error occurred."}
                    <button onClick={() => void search()}>Retry</button>
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
                                    onClick={() => { resetFiltersToDefaults(); void search(); }}
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
