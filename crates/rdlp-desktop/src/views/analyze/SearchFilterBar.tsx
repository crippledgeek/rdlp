// SearchFilterBar: dynamic filter controls (ordering, period, category) for search results.
// Fetches available filters from the backend based on selected site.

import { useQuery } from "@tanstack/react-query";
import { useStore } from "@tanstack/react-store";
import { searchStore, setSearchParam } from "@/stores/searchStore";
import { queryKeys } from "@/query/queryKeys";
import { getSearchFilters } from "@/lib/tauri";
import {
    Select,
    SelectItem,
    SelectListBox,
    SelectPopover,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import type { SearchFilter, SearchFilterDescriptor } from "@/types";

const NONE_SENTINEL = "none";

export function SearchFilterBar() {
    const site = useStore(searchStore, (s) => s.site);
    const filters = useStore(searchStore, (s) => s.filters);

    const { data: descriptors = [] } = useQuery({
        queryKey: queryKeys.filters(site),
        queryFn: () => getSearchFilters(site),
        enabled: site.length > 0,
        staleTime: Infinity,
    });

    if (descriptors.length === 0) return null;

    const getFilterValue = (key: string): string => {
        const f = filters.find((f) => f.key === key);
        return f?.value ?? "";
    };

    const setFilter = (key: string, value: string) => {
        const updated: SearchFilter[] = filters.filter((f) => f.key !== key);
        if (value) {
            updated.push({ key, value });
        }
        setSearchParam("filters", updated);
        setSearchParam("hasUserFilters", updated.length > 0);
    };

    return (
        <div className="flex items-center gap-2 px-3 py-1.5 border-b border-[#1a1a2e] shrink-0">
            <span className="text-[10px] text-[var(--text-muted)] uppercase tracking-[1px] mr-1">Filters</span>
            {descriptors.map((desc: SearchFilterDescriptor) => (
                <Select
                    key={desc.key}
                    selectedKey={getFilterValue(desc.key) || NONE_SENTINEL}
                    onSelectionChange={(key) => setFilter(desc.key, key === NONE_SENTINEL ? "" : String(key))}
                    aria-label={desc.display_name}
                >
                    <SelectTrigger className="h-6 min-h-0 px-2 py-0 text-[11px] bg-[var(--surface-elevated)] border-[#2a2a3e] rounded-[4px] text-[#aaaaaa] gap-1">
                        <SelectValue />
                    </SelectTrigger>
                    <SelectPopover>
                        <SelectListBox>
                            <SelectItem id={NONE_SENTINEL} textValue={`Any ${desc.display_name}`}>
                                Any {desc.display_name}
                            </SelectItem>
                            {desc.allowed_values.map((v) => (
                                <SelectItem key={v.value} id={v.value} textValue={v.label}>
                                    {v.label}
                                </SelectItem>
                            ))}
                        </SelectListBox>
                    </SelectPopover>
                </Select>
            ))}
        </div>
    );
}
