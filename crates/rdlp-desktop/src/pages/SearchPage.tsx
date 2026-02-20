import { useState } from "react";
import { SearchBar } from "../components/SearchBar";
import { ResultCard } from "../components/ResultCard";
import { FormatDialog } from "../components/FormatDialog";
import { useSearchStore, useQueueStore } from "../lib/store";
import type { DownloadOptions } from "../types";

/** Search page composing the search bar, results grid, and format dialog. */
export function SearchPage() {
    const status = useSearchStore((s) => s.status);
    const results = useSearchStore((s) => s.results);
    const error = useSearchStore((s) => s.error);
    const search = useSearchStore((s) => s.search);
    const hasUserFilters = useSearchStore((s) => s.hasUserFilters);
    const resetFiltersToDefaults = useSearchStore((s) => s.resetFiltersToDefaults);
    const startDownload = useQueueStore((s) => s.startDownload);

    const [formatDialogUrl, setFormatDialogUrl] = useState<string | null>(null);

    const handleDownload = (url: string) => {
        void startDownload(url);
    };

    const handleDownloadWithOptions = (url: string, options: Partial<DownloadOptions>) => {
        void startDownload(url, options);
    };

    const handleOpenFormatDialog = (url: string) => {
        setFormatDialogUrl(url);
    };

    const handleFormatDialogConfirm = (options: DownloadOptions) => {
        if (formatDialogUrl) {
            void startDownload(formatDialogUrl, options);
        }
        setFormatDialogUrl(null);
    };

    return (
        <div className="search-page">
            <SearchBar />

            {status === "loading" && (
                <div className="status-message">
                    <div className="loading-dots">
                        <span />
                        <span />
                        <span />
                    </div>
                    <p>Searching...</p>
                </div>
            )}

            {status === "error" && (
                <div className="error-banner">
                    <p>
                        {error ?? "An unknown error occurred."}
                    </p>
                    <button
                        onClick={() => void search()}
                    >
                        Retry
                    </button>
                </div>
            )}

            {status === "empty" && (
                <div className="status-message">
                    <svg className="status-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                        <circle cx="11" cy="11" r="8" />
                        <path d="M21 21l-4.3-4.3" />
                        <line x1="8" y1="11" x2="14" y2="11" />
                    </svg>
                    <p>No results found.</p>
                    {hasUserFilters && (
                        <button
                            className="clear-filters-btn"
                            onClick={() => {
                                resetFiltersToDefaults();
                                void search();
                            }}
                        >
                            Clear filters and retry
                        </button>
                    )}
                </div>
            )}

            {status === "idle" && (
                <div className="status-message">
                    <svg className="status-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                        <circle cx="11" cy="11" r="8" />
                        <path d="M21 21l-4.3-4.3" />
                    </svg>
                    <p>Search for videos to get started.</p>
                </div>
            )}

            {status === "results" && (
                <div className="results-grid">
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
