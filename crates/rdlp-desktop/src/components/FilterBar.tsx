// Chip-styled filter bar extracted from SearchBar.
// Compact 28px controls with active-state highlighting.

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

/** Compact chip-styled filter bar with default + advanced filters. */
export function FilterBar() {
    const filters = useSearchStore((s) => s.filters);
    const setFilters = useSearchStore((s) => s.setFilters);
    const filterDescriptors = useSearchStore((s) => s.filterDescriptors);
    const loadFilters = useSearchStore((s) => s.loadFilters);
    const resetFiltersToDefaults = useSearchStore((s) => s.resetFiltersToDefaults);
    const search = useSearchStore((s) => s.search);
    const status = useSearchStore((s) => s.status);
    const site = useSearchStore((s) => s.site);
    const query = useSearchStore((s) => s.query);
    const hasUserFilters = useSearchStore((s) => s.hasUserFilters);

    const [showAdvanced, setShowAdvanced] = useState(false);

    // Load filter descriptors when site changes
    useEffect(() => {
        if (site !== "") {
            void loadFilters(site);
        }
    }, [site, loadFilters]);

    // Reset advanced panel on site change
    useEffect(() => {
        setShowAdvanced(false);
    }, [site]);

    // Auto-search when filters change (only if results are showing)
    useEffect(() => {
        if (!hasUserFilters) return;
        if (status !== "results" && status !== "empty") return;
        if (query.trim() === "" || site === "") return;
        void search();
    }, [filters, hasUserFilters]); // eslint-disable-line react-hooks/exhaustive-deps

    const handleFilterChange = (key: string, value: string) => {
        const updated: SearchFilter[] = filters.filter((f) => f.key !== key);
        if (value !== "") {
            updated.push({ key, value });
        }
        setFilters(updated);
    };

    const getFilterValue = (key: string): string => {
        const filter = filters.find((f) => f.key === key);
        return filter?.value ?? "";
    };

    const defaultFilters = filterDescriptors.filter((d) => DEFAULT_FILTER_KEYS.has(d.key));
    const advancedFilters = filterDescriptors.filter((d) => !DEFAULT_FILTER_KEYS.has(d.key));
    const activeAdvancedCount = advancedFilters.filter(
        (d) => isFilterActive(getFilterValue(d.key), d.default),
    ).length;
    const hasAnyActiveFilter = [...defaultFilters, ...advancedFilters].some(
        (d) => isFilterActive(getFilterValue(d.key), d.default),
    );

    if (filterDescriptors.length === 0) return null;

    return (
        <div className="filter-bar">
            <div className="filter-bar-chips">
                {defaultFilters.map((desc) => {
                    const value = getFilterValue(desc.key);
                    const active = isFilterActive(value, desc.default);
                    return (
                        <select
                            key={desc.key}
                            className={`filter-chip${active ? " filter-chip-active" : ""}`}
                            value={value}
                            onChange={(e) => handleFilterChange(desc.key, e.target.value)}
                            disabled={status === "loading"}
                            aria-label={desc.display_name}
                        >
                            <option value="">
                                {desc.display_name}
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
                        className={`filter-chip-toggle ${showAdvanced ? "active" : ""}`}
                        onClick={() => setShowAdvanced(!showAdvanced)}
                        aria-expanded={showAdvanced}
                        aria-label="Advanced filters"
                    >
                        <svg viewBox="0 0 24 24" width="11" height="11" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <circle cx="12" cy="12" r="3" />
                            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                        </svg>
                        {activeAdvancedCount > 0 && (
                            <span className="filter-chip-badge">
                                +{activeAdvancedCount}
                            </span>
                        )}
                    </button>
                )}

                {hasAnyActiveFilter && (
                    <button
                        type="button"
                        className="filter-chip-reset"
                        onClick={resetFiltersToDefaults}
                        aria-label="Reset all filters"
                        title="Reset all filters"
                    >
                        <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                            <path d="M18 6L6 18" />
                            <path d="M6 6l12 12" />
                        </svg>
                    </button>
                )}
            </div>

            {showAdvanced && advancedFilters.length > 0 && (
                <div className="filter-bar-advanced">
                    {advancedFilters.map((desc) => {
                        const value = getFilterValue(desc.key);
                        const active = isFilterActive(value, desc.default);
                        return (
                            <div key={desc.key} className="filter-bar-advanced-item">
                                <label className="filter-bar-label">
                                    {desc.display_name}
                                </label>
                                <select
                                    className={`filter-chip${active ? " filter-chip-active" : ""}`}
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
