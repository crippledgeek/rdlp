// Chip-styled filter bar extracted from SearchBar.
// Compact 28px controls with active-state highlighting.

import { useEffect, useState } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { Settings, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { searchParamsAtom } from "../stores/searchParamsStore";
import { filtersQueryOptions } from "../api/search";
import type { SearchFilter } from "../types";

/** Sentinel value for "no selection" since Radix Select does not support empty string values. */
const NONE_SENTINEL = "none";

const DEFAULT_FILTER_KEYS = new Set([
    "sort", "quality", "orientations", "date", "min-duration", "max-duration",
    "ordering", "period",
]);

/** Check if a filter value differs from its descriptor default. */
function isFilterActive(value: string, defaultValue: string | null): boolean {
    if (value === "") return false;
    return value !== (defaultValue ?? "");
}

interface FilterBarProps {
    isFetching: boolean;
    hasSearchData: boolean;
    onSearch: () => void;
}

/** Compact chip-styled filter bar with default + advanced filters. */
export function FilterBar({ isFetching, hasSearchData, onSearch }: FilterBarProps) {
    const filters = useStore(searchParamsAtom, (s) => s.filters);
    const site = useStore(searchParamsAtom, (s) => s.site);
    const query = useStore(searchParamsAtom, (s) => s.query);
    const hasUserFilters = useStore(searchParamsAtom, (s) => s.hasUserFilters);

    const { data: filterDescriptors = [] } = useQuery(filtersQueryOptions(site));

    const [showAdvanced, setShowAdvanced] = useState(false);

    // Reset filters to defaults when filter descriptors change (new site)
    useEffect(() => {
        if (filterDescriptors.length === 0) return;
        const defaults: SearchFilter[] = [];
        for (const desc of filterDescriptors) {
            if (desc.default !== null) {
                defaults.push({ key: desc.key, value: desc.default });
            }
        }
        searchParamsAtom.setState((prev) => ({
            ...prev,
            filters: defaults,
            hasUserFilters: false,
        }));
    }, [filterDescriptors]);

    // Reset advanced panel on site change
    useEffect(() => {
        setShowAdvanced(false);
    }, [site]);

    // Auto-search when filters change (only if results are already showing)
    useEffect(() => {
        if (!hasUserFilters) return;
        // Only auto-search if a search has already been performed
        if (!hasSearchData) return;
        if (query.trim() === "" || site === "") return;
        onSearch();
    }, [filters, hasUserFilters]); // eslint-disable-line react-hooks/exhaustive-deps

    const handleFilterChange = (key: string, rawValue: string) => {
        const value = rawValue === NONE_SENTINEL ? "" : rawValue;
        const updated: SearchFilter[] = filters.filter((f) => f.key !== key);
        if (value !== "") {
            updated.push({ key, value });
        }
        searchParamsAtom.setState((prev) => ({
            ...prev,
            filters: updated,
            hasUserFilters: true,
        }));
    };

    const resetFiltersToDefaults = () => {
        const defaults: SearchFilter[] = [];
        for (const desc of filterDescriptors) {
            if (desc.default !== null) {
                defaults.push({ key: desc.key, value: desc.default });
            }
        }
        searchParamsAtom.setState((prev) => ({
            ...prev,
            filters: defaults,
            hasUserFilters: false,
        }));
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
        <div className="pt-1.5">
            <div className="flex items-center gap-1 flex-wrap">
                {defaultFilters.map((desc) => {
                    const value = getFilterValue(desc.key);
                    const active = isFilterActive(value, desc.default);
                    return (
                        <Select
                            key={desc.key}
                            value={value === "" ? NONE_SENTINEL : value}
                            onValueChange={(val) => handleFilterChange(desc.key, val)}
                            disabled={isFetching}
                        >
                            <SelectTrigger
                                className={cn(
                                    "h-7 w-auto rounded-md border border-border/50 bg-transparent px-2.5 text-[11px] font-medium text-muted-foreground cursor-pointer transition-colors",
                                    "hover:border-white/[0.08] hover:text-foreground/70",
                                    "focus:outline-none focus:ring-1 focus:ring-ring",
                                    "disabled:opacity-40 disabled:cursor-not-allowed",
                                    active && "border-primary/30 text-foreground bg-primary/[0.06]",
                                )}
                                aria-label={desc.display_name}
                            >
                                <SelectValue placeholder={desc.display_name} />
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value={NONE_SENTINEL}>
                                    {desc.display_name}
                                </SelectItem>
                                {desc.allowed_values.map((v) => (
                                    <SelectItem key={v.value} value={v.value}>
                                        {v.label}
                                    </SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                    );
                })}

                {advancedFilters.length > 0 && (
                    <Button
                        variant="outline"
                        type="button"
                        className={cn(
                            "h-7 px-2.5 gap-1 text-[11px] border-border/50 text-muted-foreground",
                            showAdvanced && "border-primary text-primary bg-primary/10",
                        )}
                        onClick={() => setShowAdvanced(!showAdvanced)}
                        aria-expanded={showAdvanced}
                        aria-label="Advanced filters"
                    >
                        <Settings className="h-[11px] w-[11px]" />
                        {activeAdvancedCount > 0 && (
                            <span className="text-[10px] font-bold text-primary">
                                +{activeAdvancedCount}
                            </span>
                        )}
                    </Button>
                )}

                {hasAnyActiveFilter && (
                    <Button
                        variant="ghost"
                        size="icon"
                        type="button"
                        className="h-7 w-7 text-muted-foreground hover:text-foreground"
                        onClick={resetFiltersToDefaults}
                        aria-label="Reset all filters"
                        title="Reset all filters"
                    >
                        <X className="h-2.5 w-2.5" />
                    </Button>
                )}
            </div>

            {showAdvanced && advancedFilters.length > 0 && (
                <div className="flex flex-wrap gap-2.5 mt-1.5 p-2.5 bg-card border border-border rounded-md animate-in fade-in duration-200">
                    {advancedFilters.map((desc) => {
                        const value = getFilterValue(desc.key);
                        const active = isFilterActive(value, desc.default);
                        return (
                            <div key={desc.key} className="flex flex-col gap-[3px]">
                                <label className="text-[10px] font-semibold text-muted-foreground tracking-wide uppercase">
                                    {desc.display_name}
                                </label>
                                <Select
                                    value={value === "" ? NONE_SENTINEL : value}
                                    onValueChange={(val) => handleFilterChange(desc.key, val)}
                                    disabled={isFetching}
                                >
                                    <SelectTrigger
                                        className={cn(
                                            "h-7 w-auto rounded-md border border-border/50 bg-transparent px-2.5 text-[11px] font-medium text-muted-foreground cursor-pointer transition-colors",
                                            "hover:border-white/[0.08] hover:text-foreground/70",
                                            "focus:outline-none focus:ring-1 focus:ring-ring",
                                            "disabled:opacity-40 disabled:cursor-not-allowed",
                                            active && "border-primary/30 text-foreground bg-primary/[0.06]",
                                        )}
                                        aria-label={desc.display_name}
                                    >
                                        <SelectValue placeholder="Any" />
                                    </SelectTrigger>
                                    <SelectContent>
                                        <SelectItem value={NONE_SENTINEL}>Any</SelectItem>
                                        {desc.allowed_values.map((v) => (
                                            <SelectItem key={v.value} value={v.value}>
                                                {v.label}
                                            </SelectItem>
                                        ))}
                                    </SelectContent>
                                </Select>
                            </div>
                        );
                    })}
                </div>
            )}
        </div>
    );
}
