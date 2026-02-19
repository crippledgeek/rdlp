import { SearchBar } from "../components/SearchBar";
import { ResultCard } from "../components/ResultCard";
import { useSearchStore } from "../lib/store";
import { useQueueStore } from "../lib/store";

/** Search page composing the search bar and results grid. */
export function SearchPage() {
    const status = useSearchStore((s) => s.status);
    const results = useSearchStore((s) => s.results);
    const error = useSearchStore((s) => s.error);
    const search = useSearchStore((s) => s.search);
    const startDownload = useQueueStore((s) => s.startDownload);

    const handleDownload = (url: string) => {
        void startDownload(url);
    };

    return (
        <div className="search-page">
            <SearchBar />

            {status === "loading" && (
                <div className="status-message">
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
                    <p>No results found.</p>
                </div>
            )}

            {status === "results" && (
                <div className="results-grid">
                    {results.map((result) => (
                        <ResultCard
                            key={result.video_url}
                            result={result}
                            onDownload={handleDownload}
                        />
                    ))}
                </div>
            )}
        </div>
    );
}
