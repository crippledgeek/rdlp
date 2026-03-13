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
import { useSearchPage } from "../hooks/useSearchPage";
import type { ViewMode } from "../types";

interface SearchPageProps {
    activeTab: string;
    viewMode: ViewMode;
}

/** Main search page composing command bar, filters, results, and dialogs. */
export function SearchPage({ activeTab, viewMode }: SearchPageProps) {
    const {
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
    } = useSearchPage({ activeTab });

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
