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
    /** Whether the user has picked a site themselves.
     *
     *  `site` alone cannot answer that: `""` is both the untouched resting
     *  value and what CommandBar writes when the user deliberately selects
     *  "All sites". Without this flag a deliberate "All sites" is
     *  indistinguishable from "not chosen yet" and gets overwritten by the
     *  seeded default (#589) — the same collapse the `Vec<String>` subtitle
     *  field had on the Rust side. */
    siteTouched: boolean;
}

const initialState: SearchParams = {
    query: "",
    site: "",
    filters: [],
    hasUserFilters: false,
    siteTouched: false,
};

export const searchStore = new Store<SearchParams>(initialState);

/** Set a single field on the search params store. */
export function setSearchParam<K extends keyof SearchParams>(
    key: K,
    value: SearchParams[K],
): void {
    searchStore.setState((prev) => ({ ...prev, [key]: value }));
}

/** Record a user-chosen search site, marking the selector as touched.
 *
 * Use this from UI handlers rather than `setSearchParam("site", …)`: the two
 * fields must move together, or an "All sites" pick is silently re-seeded.
 * `""` means "All sites".
 */
export function setSearchSite(site: string): void {
    searchStore.setState((prev) => ({ ...prev, site, siteTouched: true }));
}

/** Reset the search form to initial state. */
export function resetSearchParams(): void {
    searchStore.setState(() => ({ ...initialState }));
}

// Re-export legacy atom name for backward compatibility during migration.
export { searchStore as searchParamsAtom };
