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
                className="search-site-select"
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

            <input
                className="search-input"
                type="text"
                placeholder="Search videos..."
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                disabled={status === "loading"}
            />

            <button
                className="search-button"
                type="submit"
                disabled={isDisabled}
            >
                {status === "loading" ? "Searching..." : "Search"}
            </button>
        </form>
    );
}
