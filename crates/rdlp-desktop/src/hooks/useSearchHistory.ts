// Custom hook for persisting search history to localStorage.
//
// Stores the last 30 searches grouped by site. Supports add, remove,
// clear, and restore operations. Deduplicates by query+site.

import { useCallback, useSyncExternalStore } from "react";

const STORAGE_KEY = "rdlp-search-history";
const MAX_ENTRIES = 30;

export interface SearchHistoryEntry {
    query: string;
    site: string;
    siteDisplayName: string;
    filters: Array<{ key: string; value: string }>;
    timestamp: number;
}

type Listener = () => void;
const listeners = new Set<Listener>();

function emitChange() {
    for (const listener of listeners) {
        listener();
    }
}

function getSnapshot(): SearchHistoryEntry[] {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return [];
        return JSON.parse(raw) as SearchHistoryEntry[];
    } catch {
        return [];
    }
}

function save(entries: SearchHistoryEntry[]) {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
    emitChange();
}

function subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
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
    const entries = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

    const addEntry = useCallback(
        (entry: Omit<SearchHistoryEntry, "timestamp">) => {
            const current = getSnapshot();
            const filtered = current.filter(
                (e) => !(e.query === entry.query && e.site === entry.site),
            );
            const newEntry: SearchHistoryEntry = {
                ...entry,
                timestamp: Date.now(),
            };
            const updated = [newEntry, ...filtered].slice(0, MAX_ENTRIES);
            save(updated);
        },
        [],
    );

    const removeEntry = useCallback((query: string, site: string) => {
        const current = getSnapshot();
        save(current.filter((e) => !(e.query === query && e.site === site)));
    }, []);

    const clearAll = useCallback(() => {
        save([]);
    }, []);

    const grouped = groupBySite(entries);

    return { entries, grouped, addEntry, removeEntry, clearAll };
}
