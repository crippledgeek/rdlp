// TanStack Store for queue filter state.

import { Store } from "@tanstack/store";

export type QueueFilter = "all" | "active" | "completed" | "failed";

export interface QueueFilterState {
    filter: QueueFilter;
}

export const queueFilterStore = new Store<QueueFilterState>({ filter: "all" });

export function setQueueFilter(filter: QueueFilter): void {
    queueFilterStore.setState(() => ({ filter }));
}
