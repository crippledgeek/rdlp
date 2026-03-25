// QueueNav: nav panel content for the Queue view.
// Filter toggle buttons, bulk actions, and queue stats.

import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { Trash2 } from "lucide-react";
import { queueFilterStore, setQueueFilter } from "@/stores/queueFilterStore";
import { downloadsQueryOptions, clearCompletedJobs } from "@/api/downloads";
import { cn } from "@/lib/utils";
import type { JobStatus } from "@/types";

type FilterKey = "all" | "active" | "completed" | "failed";

const filters: { key: FilterKey; label: string; statuses?: JobStatus[] }[] = [
    { key: "all", label: "All" },
    { key: "active", label: "Active", statuses: ["running", "pending"] },
    { key: "completed", label: "Completed", statuses: ["completed", "cancelled"] },
    { key: "failed", label: "Failed", statuses: ["failed"] },
];

export function QueueNav() {
    const activeFilter = useStore(queueFilterStore, (s) => s.filter);
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    const counts: Record<FilterKey, number> = {
        all: jobs.length,
        active: jobs.filter((j) => j.status === "running" || j.status === "pending").length,
        completed: jobs.filter((j) => j.status === "completed" || j.status === "cancelled").length,
        failed: jobs.filter((j) => j.status === "failed").length,
    };

    const handleClearCompleted = () => {
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
                        return (
                            <button
                                key={key}
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
                                            isActive ? "text-[#4a9eff]" : "text-[#444444]",
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
                <div className="px-3 space-y-1">
                    <div className="flex justify-between text-[11px]">
                        <span className="text-[#666666]">Total</span>
                        <span className="text-[#aaaaaa] font-mono">{jobs.length}</span>
                    </div>
                    <div className="flex justify-between text-[11px]">
                        <span className="text-[#666666]">Active</span>
                        <span className="text-[#e8a838] font-mono">{counts.active}</span>
                    </div>
                    <div className="flex justify-between text-[11px]">
                        <span className="text-[#666666]">Completed</span>
                        <span className="text-[#4a9e4a] font-mono">{counts.completed}</span>
                    </div>
                    <div className="flex justify-between text-[11px]">
                        <span className="text-[#666666]">Failed</span>
                        <span className="text-[#e85858] font-mono">{counts.failed}</span>
                    </div>
                </div>
            </section>

            {/* Bulk actions */}
            {counts.completed > 0 && (
                <section className="mt-auto pb-1">
                    <button
                        onClick={handleClearCompleted}
                        className="flex items-center gap-2 w-full px-3 py-1.5 rounded-[4px] text-[12px] text-[#666666] hover:text-[#e85858] hover:bg-[#2a0a0a] transition-colors"
                    >
                        <Trash2 className="w-3 h-3" />
                        Clear Completed
                    </button>
                </section>
            )}
        </div>
    );
}
