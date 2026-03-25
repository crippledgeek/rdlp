// TanStack Store for history filter/search state.

import { Store } from "@tanstack/store";

export interface HistoryFilterState {
    search: string;
    groupBy: "date" | "site" | "type";
}

export const historyFilterStore = new Store<HistoryFilterState>({
    search: "",
    groupBy: "date",
});

export function setHistorySearch(search: string): void {
    historyFilterStore.setState((prev) => ({ ...prev, search }));
}

export function setHistoryGroupBy(groupBy: HistoryFilterState["groupBy"]): void {
    historyFilterStore.setState((prev) => ({ ...prev, groupBy }));
}
