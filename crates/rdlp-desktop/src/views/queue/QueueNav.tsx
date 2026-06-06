// QueueNav: nav panel content for the Queue view.
// Filter toggle buttons, bulk actions, and queue stats.

import { useMemo } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { queueFilterStore, setQueueFilter } from "@/stores/queueFilterStore";
import { downloadsQueryOptions, clearCompletedJobs } from "@/api/downloads";
import { countByBucket, TERMINAL_BUCKETS, type QueueFilter } from "@/lib/jobStatus";
import { cn } from "@/lib/utils";

const filters: { key: QueueFilter; label: string }[] = [
    { key: "all", label: "All" },
    { key: "active", label: "Active" },
    { key: "completed", label: "Completed" },
    { key: "cancelled", label: "Cancelled" },
    { key: "failed", label: "Failed" },
];

const statRows: { key: QueueFilter; label: string; valueClass: string }[] = [
    { key: "all", label: "Total", valueClass: "text-[#aaaaaa]" },
    { key: "active", label: "Active", valueClass: "text-[#e8a838]" },
    { key: "completed", label: "Completed", valueClass: "text-[#4a9e4a]" },
    { key: "cancelled", label: "Cancelled", valueClass: "text-[var(--text-muted)]" },
    { key: "failed", label: "Failed", valueClass: "text-[#e85858]" },
];

export function QueueNav() {
    const activeFilter = useStore(queueFilterStore, (s) => s.filter);
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    const counts = useMemo(() => countByBucket(jobs), [jobs]);
    const hasFinished = TERMINAL_BUCKETS.some((b) => counts[b] > 0);

    const handleClearFinished = () => {
        clearCompletedJobs().catch(console.error);
    };

    return (
        <div className="flex flex-col h-full overflow-y-auto p-2 gap-3">
            {/* Filter buttons */}
            <section>
                <h2 className="section-heading px-2 mb-1">Filter</h2>
                <div className="flex flex-col gap-0.5">
                    {filters.map(({ key, label }) => {
                        const count = counts[key];
                        const isActive = activeFilter === key;
                        const ariaLabel =
                            count > 0
                                ? `Filter by ${label} downloads, ${count} item${count === 1 ? "" : "s"}`
                                : `Filter by ${label} downloads`;
                        return (
                            <button
                                key={key}
                                aria-label={ariaLabel}
                                aria-pressed={isActive}
                                onClick={() => setQueueFilter(key)}
                                className={cn(
                                    "flex items-center justify-between px-3 py-1.5 rounded-[4px] text-[12px] transition-colors text-left",
                                    isActive
                                        ? "bg-[#1a2a4a] text-[#4a9eff]"
                                        : "text-[#aaaaaa] hover:bg-[#141428] hover:text-[#eeeeee]",
                                )}
                            >
                                <span>{label}</span>
                                {count > 0 && (
                                    <span
                                        className={cn(
                                            "text-[10px] font-mono",
                                            isActive ? "text-[#4a9eff]" : "text-[var(--text-muted)]",
                                        )}
                                    >
                                        {count}
                                    </span>
                                )}
                            </button>
                        );
                    })}
                </div>
            </section>

            {/* Stats */}
            <section>
                <h2 className="section-heading px-2 mb-1">Stats</h2>
                <dl className="px-3 space-y-1">
                    {statRows.map(({ key, label, valueClass }) => (
                        <div key={key} className="flex justify-between text-[11px]">
                            <dt className="text-[var(--text-muted)]">{label}</dt>
                            <dd className={cn("font-mono", valueClass)}>{counts[key]}</dd>
                        </div>
                    ))}
                </dl>
            </section>

            {/* Bulk actions */}
            {hasFinished && (
                <section className="mt-auto pb-1">
                    <button
                        onClick={handleClearFinished}
                        className="flex items-center gap-2 w-full px-3 py-1.5 rounded-[4px] text-[12px] text-[var(--text-muted)] hover:text-[#e85858] hover:bg-[#2a0a0a] transition-colors"
                    >
                        <Trash2 className="w-3 h-3" />
                        Clear Finished
                    </button>
                </section>
            )}
        </div>
    );
}
