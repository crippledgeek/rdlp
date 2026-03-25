// TanStack Store atom for search form UI state.
//
// This holds the search input fields that are NOT server state:
// query text, selected site, applied filters, and whether the user
// has manually edited filters. Components subscribe via useStore()
// with selectors for granular reactivity.

import { Store } from "@tanstack/store";
import type { SearchFilter } from "../types";

export interface SearchParams {
    query: string;
    site: string;
    filters: SearchFilter[];
    hasUserFilters: boolean;
}

const initialState: SearchParams = {
    query: "",
    site: "",
    filters: [],
    hasUserFilters: false,
};

export const searchStore = new Store<SearchParams>(initialState);

/** Set a single field on the search params store. */
export function setSearchParam<K extends keyof SearchParams>(
    key: K,
    value: SearchParams[K],
): void {
    searchStore.setState((prev) => ({ ...prev, [key]: value }));
}

/** Reset the search form to initial state. */
export function resetSearchParams(): void {
    searchStore.setState(() => ({ ...initialState }));
}

// Re-export legacy atom name for backward compatibility during migration.
export { searchStore as searchParamsAtom };
