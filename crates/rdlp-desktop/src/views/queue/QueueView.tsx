// QueueView: virtualized job list for the Queue view.
// Reads from downloads query, filters by QueueNav selection.

import { useRef, useMemo } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Download } from "lucide-react";
import { downloadsQueryOptions } from "@/api/downloads";
import { queueFilterStore } from "@/stores/queueFilterStore";
import { JobCard } from "./JobCard";
import type { DownloadJob } from "@/types";

const STATUS_ORDER: Record<string, number> = {
    running: 0,
    pending: 1,
    failed: 2,
    completed: 3,
    cancelled: 4,
};

function filterJobs(jobs: DownloadJob[], filter: string): DownloadJob[] {
    switch (filter) {
        case "active":
            return jobs.filter((j) => j.status === "running" || j.status === "pending");
        case "completed":
            return jobs.filter((j) => j.status === "completed" || j.status === "cancelled");
        case "failed":
            return jobs.filter((j) => j.status === "failed");
        default:
            return jobs;
    }
}

export function QueueView() {
    const parentRef = useRef<HTMLDivElement>(null);
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());
    const activeFilter = useStore(queueFilterStore, (s) => s.filter);

    const sorted = useMemo(() => {
        const filtered = filterJobs(jobs, activeFilter);
        return [...filtered].sort((a, b) => {
            const aOrder = STATUS_ORDER[a.status] ?? 5;
            const bOrder = STATUS_ORDER[b.status] ?? 5;
            if (aOrder !== bOrder) return aOrder - bOrder;
            return (b.started_at ?? 0) - (a.started_at ?? 0);
        });
    }, [jobs, activeFilter]);

    const virtualizer = useVirtualizer({
        count: sorted.length,
        getScrollElement: () => parentRef.current,
        estimateSize: () => 90,
        overscan: 5,
    });

    if (jobs.length === 0) {
        return (
            <div className="flex flex-col items-center justify-center h-full gap-3">
                <Download className="w-10 h-10 text-[#1a1a2e]" />
                <p className="text-[13px] text-[#444444]">No downloads yet</p>
                <p className="text-[11px] text-[#333333]">Paste a URL in the command bar to start</p>
            </div>
        );
    }

    if (sorted.length === 0) {
        return (
            <div className="flex items-center justify-center h-full">
                <p className="text-[13px] text-[#444444]">No jobs match this filter</p>
            </div>
        );
    }

    return (
        <div ref={parentRef} className="h-full overflow-y-auto">
            <div
                className="relative p-3"
                style={{ height: virtualizer.getTotalSize() + 24 }}
            >
                {virtualizer.getVirtualItems().map((virtualItem) => {
                    const job = sorted[virtualItem.index];
                    if (!job) return null;
                    return (
                        <div
                            key={job.id}
                            className="absolute left-3 right-3"
                            style={{
                                top: virtualItem.start + 12,
                                height: virtualItem.size,
                            }}
                        >
                            <JobCard job={job} />
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
