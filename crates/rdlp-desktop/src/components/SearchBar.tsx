import { useEffect, useState } from "react";
import { useSearchStore } from "../lib/store";
import type { SearchFilter } from "../types";

const DEFAULT_FILTER_KEYS = new Set([
    "sort", "quality", "orientations", "date", "min-duration", "max-duration",
]);

/** Check if a filter value differs from its descriptor default. */
function isFilterActive(value: string, defaultValue: string | null): boolean {
    if (value === "") return false;
    return value !== (defaultValue ?? "");
}

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
    const resetFiltersToDefaults = useSearchStore((s) => s.resetFiltersToDefaults);
    const search = useSearchStore((s) => s.search);
    const status = useSearchStore((s) => s.status);

    const [showAdvanced, setShowAdvanced] = useState(false);
    const hasUserFilters = useSearchStore((s) => s.hasUserFilters);

    useEffect(() => {
        void loadProviders();
    }, [loadProviders]);

    // Load filter descriptors when site changes
    useEffect(() => {
        if (site !== "") {
            void loadFilters(site);
        }
    }, [site, loadFilters]);

    // Auto-search when user changes filters (only if results are showing)
    useEffect(() => {
        if (!hasUserFilters) return;
        if (status !== "results" && status !== "empty") return;
        if (query.trim() === "" || site === "") return;
        void search();
    }, [filters, hasUserFilters]); // eslint-disable-line react-hooks/exhaustive-deps

    const isDisabled = status === "loading" || query.trim() === "";

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault();
        if (!isDisabled) {
            void search();
        }
    };

    const handleSiteChange = (newSite: string) => {
        setSite(newSite);
        setShowAdvanced(false);
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
    const defaultFilters = filterDescriptors.filter((d) => DEFAULT_FILTER_KEYS.has(d.key));
    const advancedFilters = filterDescriptors.filter((d) => !DEFAULT_FILTER_KEYS.has(d.key));
    const activeAdvancedCount = advancedFilters.filter(
        (d) => isFilterActive(getFilterValue(d.key), d.default),
    ).length;
    const hasAnyActiveFilter = [...defaultFilters, ...advancedFilters].some(
        (d) => isFilterActive(getFilterValue(d.key), d.default),
    );

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
                    {defaultFilters.map((desc) => {
                        const value = getFilterValue(desc.key);
                        const active = isFilterActive(value, desc.default);
                        return (
                            <select
                                key={desc.key}
                                className={`filter-select${active ? " filter-active" : ""}`}
                                value={value}
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
                        );
                    })}

                    {advancedFilters.length > 0 && (
                        <button
                            type="button"
                            className={`filter-toggle ${showAdvanced ? "active" : ""}`}
                            onClick={() => setShowAdvanced(!showAdvanced)}
                            aria-expanded={showAdvanced}
                            aria-label="Advanced filters"
                        >
                            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                <line x1="4" y1="21" x2="4" y2="14" />
                                <line x1="4" y1="10" x2="4" y2="3" />
                                <line x1="12" y1="21" x2="12" y2="12" />
                                <line x1="12" y1="8" x2="12" y2="3" />
                                <line x1="20" y1="21" x2="20" y2="16" />
                                <line x1="20" y1="12" x2="20" y2="3" />
                                <line x1="1" y1="14" x2="7" y2="14" />
                                <line x1="9" y1="8" x2="15" y2="8" />
                                <line x1="17" y1="16" x2="23" y2="16" />
                            </svg>
                            Filters
                            {activeAdvancedCount > 0 && (
                                <span className="filter-badge">
                                    {activeAdvancedCount}
                                </span>
                            )}
                        </button>
                    )}

                    {hasAnyActiveFilter && (
                        <button
                            type="button"
                            className="filter-reset"
                            onClick={resetFiltersToDefaults}
                            aria-label="Reset all filters"
                        >
                            <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M18 6L6 18" />
                                <path d="M6 6l12 12" />
                            </svg>
                            Reset
                        </button>
                    )}
                </div>
            )}

            {showAdvanced && advancedFilters.length > 0 && (
                <div className="filter-advanced">
                    {advancedFilters.map((desc) => {
                        const value = getFilterValue(desc.key);
                        const active = isFilterActive(value, desc.default);
                        return (
                            <div key={desc.key} className="filter-advanced-item">
                                <label className="filter-label">
                                    {desc.display_name}
                                </label>
                                <select
                                    className={`filter-select${active ? " filter-active" : ""}`}
                                    value={value}
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
                        );
                    })}
                </div>
            )}
        </div>
    );
}
