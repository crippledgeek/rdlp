import { SearchBar } from "../components/SearchBar";
import { ResultCard } from "../components/ResultCard";
import { useSearchStore, useQueueStore } from "../lib/store";

/** Search page composing the search bar and results grid. */
export function SearchPage() {
    const status = useSearchStore((s) => s.status);
    const results = useSearchStore((s) => s.results);
    const error = useSearchStore((s) => s.error);
    const search = useSearchStore((s) => s.search);
    const filters = useSearchStore((s) => s.filters);
    const setFilters = useSearchStore((s) => s.setFilters);
    const startDownload = useQueueStore((s) => s.startDownload);

    const handleDownload = (url: string) => {
        void startDownload(url);
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
                    {filters.length > 0 && (
                        <button
                            className="clear-filters-btn"
                            onClick={() => {
                                setFilters([]);
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
                        />
                    ))}
                </div>
            )}
        </div>
    );
}
