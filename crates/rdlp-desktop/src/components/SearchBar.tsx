import { useEffect } from "react";
import { useSearchStore } from "../lib/store";

/** Search form with site selector, query input, and submit button. */
export function SearchBar() {
    const query = useSearchStore((s) => s.query);
    const setQuery = useSearchStore((s) => s.setQuery);
    const site = useSearchStore((s) => s.site);
    const setSite = useSearchStore((s) => s.setSite);
    const providers = useSearchStore((s) => s.providers);
    const loadProviders = useSearchStore((s) => s.loadProviders);
    const search = useSearchStore((s) => s.search);
    const status = useSearchStore((s) => s.status);

    useEffect(() => {
        void loadProviders();
    }, [loadProviders]);

    const isDisabled = status === "loading" || query.trim() === "";

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault();
        if (!isDisabled) {
            void search();
        }
    };

    return (
        <form className="search-bar" onSubmit={handleSubmit}>
            <select
                className="site-selector"
                value={site}
                onChange={(e) => setSite(e.target.value)}
                disabled={status === "loading"}
            >
                {providers.length === 0 && (
                    <option value="">Loading sites...</option>
                )}
                {providers.map((p) => (
                    <option key={p.name} value={p.name}>
                        {p.display_name}
                    </option>
                ))}
            </select>

            <div className="search-input-wrapper">
                <svg className="search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <circle cx="11" cy="11" r="8" />
                    <path d="M21 21l-4.3-4.3" />
                </svg>
                <input
                    className="search-input"
                    type="text"
                    placeholder="Search videos..."
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    disabled={status === "loading"}
                />
            </div>

            <button
                type="submit"
                disabled={isDisabled}
            >
                {status === "loading" ? "Searching..." : "Search"}
            </button>
        </form>
    );
}
