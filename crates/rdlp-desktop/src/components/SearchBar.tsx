import { useEffect, useState } from "react";
import { useSearchStore } from "../lib/store";
import type { SearchFilter } from "../types";

/** Search form with site selector, query input, filters, and submit button. */
export function SearchBar() {
    const query = useSearchStore((s) => s.query);
    const setQuery = useSearchStore((s) => s.setQuery);
    const site = useSearchStore((s) => s.site);
    const setSite = useSearchStore((s) => s.setSite);
    const providers = useSearchStore((s) => s.providers);
    const loadProviders = useSearchStore((s) => s.loadProviders);
    const filters = useSearchStore((s) => s.filters);
    const setFilters = useSearchStore((s) => s.setFilters);
    const filterDescriptors = useSearchStore((s) => s.filterDescriptors);
    const loadFilters = useSearchStore((s) => s.loadFilters);
    const search = useSearchStore((s) => s.search);
    const status = useSearchStore((s) => s.status);

    const [showAdvanced, setShowAdvanced] = useState(false);

    useEffect(() => {
        void loadProviders();
    }, [loadProviders]);

    // Load filter descriptors when site changes
    useEffect(() => {
        if (site !== "") {
            void loadFilters(site);
        }
    }, [site, loadFilters]);

    const isDisabled = status === "loading" || query.trim() === "";

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault();
        if (!isDisabled) {
            void search();
        }
    };

    const handleSiteChange = (newSite: string) => {
        setSite(newSite);
        // loadFilters will fire via the useEffect above, resetting filters to defaults
    };

    const handleFilterChange = (key: string, value: string) => {
        const updated: SearchFilter[] = filters.filter((f) => f.key !== key);
        // Only add filter if value is non-empty (empty = "any"/unset)
        if (value !== "") {
            updated.push({ key, value });
        }
        setFilters(updated);
    };

    const getFilterValue = (key: string): string => {
        const filter = filters.find((f) => f.key === key);
        return filter?.value ?? "";
    };

    // Split descriptors into default (sort, quality, duration) and advanced
    const defaultKeys = new Set(["sort", "quality", "duration-min", "duration-max"]);
    const defaultFilters = filterDescriptors.filter((d) => defaultKeys.has(d.key));
    const advancedFilters = filterDescriptors.filter((d) => !defaultKeys.has(d.key));
    const activeAdvancedCount = advancedFilters.filter(
        (d) => {
            const val = getFilterValue(d.key);
            return val !== "" && val !== (d.default ?? "");
        },
    ).length;

    return (
        <div className="search-bar-container">
            <form className="search-bar" onSubmit={handleSubmit}>
                <select
                    className="site-selector"
                    value={site}
                    onChange={(e) => handleSiteChange(e.target.value)}
                    disabled={status === "loading"}
                    aria-label="Search site"
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

            {filterDescriptors.length > 0 && (
                <div className="filter-row">
                    {defaultFilters.map((desc) => (
                        <select
                            key={desc.key}
                            className="filter-select"
                            value={getFilterValue(desc.key)}
                            onChange={(e) => handleFilterChange(desc.key, e.target.value)}
                            disabled={status === "loading"}
                            aria-label={desc.display_name}
                        >
                            <option value="">
                                {desc.display_name}: Any
                            </option>
                            {desc.allowed_values.map((v) => (
                                <option key={v.value} value={v.value}>
                                    {v.label}
                                </option>
                            ))}
                        </select>
                    ))}

                    {advancedFilters.length > 0 && (
                        <button
                            type="button"
                            className={`filter-toggle ${showAdvanced ? "active" : ""}`}
                            onClick={() => setShowAdvanced(!showAdvanced)}
                            aria-expanded={showAdvanced}
                            aria-label={`${activeAdvancedCount} active filters`}
                        >
                            Filters
                            {activeAdvancedCount > 0 && (
                                <span className="filter-badge">
                                    {activeAdvancedCount}
                                </span>
                            )}
                        </button>
                    )}
                </div>
            )}

            {showAdvanced && advancedFilters.length > 0 && (
                <div className="filter-advanced">
                    {advancedFilters.map((desc) => (
                        <div key={desc.key} className="filter-advanced-item">
                            <label className="filter-label">
                                {desc.display_name}
                            </label>
                            <select
                                className="filter-select"
                                value={getFilterValue(desc.key)}
                                onChange={(e) => handleFilterChange(desc.key, e.target.value)}
                                disabled={status === "loading"}
                                aria-label={desc.display_name}
                            >
                                <option value="">Any</option>
                                {desc.allowed_values.map((v) => (
                                    <option key={v.value} value={v.value}>
                                        {v.label}
                                    </option>
                                ))}
                            </select>
                        </div>
                    ))}
                </div>
            )}
        </div>
    );
}
