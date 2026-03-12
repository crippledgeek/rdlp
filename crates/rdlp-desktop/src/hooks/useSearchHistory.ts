// Custom hook for persisting search history to localStorage.
//
// Stores the last 30 searches grouped by site. Supports add, remove,
// clear, and restore operations. Deduplicates by query+site.

import { useLocalStorage } from "usehooks-ts";

const STORAGE_KEY = "rdlp-search-history";
const MAX_ENTRIES = 30;

export interface SearchHistoryEntry {
    query: string;
    site: string;
    siteDisplayName: string;
    filters: Array<{ key: string; value: string }>;
    timestamp: number;
}

export interface SiteGroup {
    site: string;
    displayName: string;
    entries: SearchHistoryEntry[];
}

function groupBySite(entries: SearchHistoryEntry[]): SiteGroup[] {
    const map = new Map<string, SiteGroup>();
    for (const entry of entries) {
        let group = map.get(entry.site);
        if (!group) {
            group = {
                site: entry.site,
                displayName: entry.siteDisplayName,
                entries: [],
            };
            map.set(entry.site, group);
        }
        group.entries.push(entry);
    }
    return Array.from(map.values());
}

/** Hook providing search history with add/remove/clear operations. */
export function useSearchHistory() {
    const [entries, setEntries] = useLocalStorage<SearchHistoryEntry[]>(STORAGE_KEY, []);

    const addEntry = (entry: Omit<SearchHistoryEntry, "timestamp">) => {
        setEntries((current) => {
            const filtered = current.filter(
                (e) => !(e.query === entry.query && e.site === entry.site),
            );
            const newEntry: SearchHistoryEntry = {
                ...entry,
                timestamp: Date.now(),
            };
            return [newEntry, ...filtered].slice(0, MAX_ENTRIES);
        });
    };

    const removeEntry = (query: string, site: string) => {
        setEntries((current) =>
            current.filter((e) => !(e.query === query && e.site === site)),
        );
    };

    const clearAll = () => {
        setEntries([]);
    };

    const grouped = groupBySite(entries);

    return { entries, grouped, addEntry, removeEntry, clearAll };
}
