// TanStack Store for queue filter state.

import { Store } from "@tanstack/store";
import type { QueueFilter } from "@/lib/jobStatus";

export type { QueueFilter };

export interface QueueFilterState {
    filter: QueueFilter;
}

export const queueFilterStore = new Store<QueueFilterState>({ filter: "all" });

export function setQueueFilter(filter: QueueFilter): void {
    queueFilterStore.setState(() => ({ filter }));
}
