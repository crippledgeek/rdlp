// HistoryView: completed downloads list with search and grouping.
// Session-only — no SQLite persistence yet.

import { useMemo } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { Clock } from "lucide-react";
import { downloadsQueryOptions } from "@/api/downloads";
import { isTerminal } from "@/lib/jobStatus";
import { historyFilterStore } from "@/stores/historyFilterStore";
import { HistoryRow } from "./HistoryRow";
import type { DownloadJob } from "@/types";

type GroupBy = "date" | "site" | "type";

function extractSite(url: string): string {
    try {
        return new URL(url).hostname.replace(/^www\./, "");
    } catch {
        return "unknown";
    }
}

function groupJobs(
    jobs: DownloadJob[],
    groupBy: GroupBy,
): { label: string; jobs: DownloadJob[] }[] {
    const map = new Map<string, DownloadJob[]>();

    for (const job of jobs) {
        let key: string;
        if (groupBy === "site") {
            key = extractSite(job.url);
        } else if (groupBy === "type") {
            key = job.status === "completed" ? "Completed" : job.status === "failed" ? "Failed" : "Cancelled";
        } else {
            const ts = job.completed_at ?? job.started_at;
            key = ts
                ? new Date(ts * 1000).toLocaleDateString(undefined, {
                      weekday: "long",
                      month: "short",
                      day: "numeric",
                  })
                : "Unknown";
        }
        const arr = map.get(key);
        if (arr) arr.push(job);
        else map.set(key, [job]);
    }

    return Array.from(map.entries()).map(([label, jobs]) => ({ label, jobs }));
}

export function HistoryView() {
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());
    const { search, groupBy } = useStore(historyFilterStore, (s) => s);

    const terminalJobs = useMemo(
        () =>
            jobs
                .filter((j) => isTerminal(j.status))
                .sort(
                    (a, b) =>
                        (b.completed_at ?? b.started_at ?? 0) -
                        (a.completed_at ?? a.started_at ?? 0),
                ),
        [jobs],
    );

    const filtered = useMemo(() => {
        if (!search.trim()) return terminalJobs;
        const q = search.toLowerCase();
        return terminalJobs.filter(
            (j) =>
                (j.title ?? "").toLowerCase().includes(q) ||
                j.url.toLowerCase().includes(q) ||
                extractSite(j.url).includes(q),
        );
    }, [terminalJobs, search]);

    const groups = useMemo(() => groupJobs(filtered, groupBy), [filtered, groupBy]);

    if (terminalJobs.length === 0) {
        return (
            <div className="flex flex-col items-center justify-center h-full gap-3">
                <Clock className="w-10 h-10 text-[#1a1a2e]" />
                <p className="text-[13px] text-[var(--text-muted)]">No download history</p>
                <p className="text-[11px] text-[var(--text-muted)]">Completed downloads will appear here</p>
                <p className="text-[10px] text-[var(--text-muted)]">History is available for this session only</p>
            </div>
        );
    }

    if (filtered.length === 0) {
        return (
            <div className="flex items-center justify-center h-full">
                <p className="text-[13px] text-[var(--text-muted)]">No results match your search</p>
            </div>
        );
    }

    return (
        <div className="flex flex-col h-full overflow-y-auto">
            {groups.map(({ label, jobs: groupJobs }) => (
                <div key={label}>
                    <div className="sticky top-0 z-10 px-3 py-1.5 bg-[var(--surface-base)] border-b border-[#0f0f1e]">
                        <p className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider font-medium">
                            {label}
                            <span className="ml-1.5 text-[var(--text-muted)] normal-case">{groupJobs.length}</span>
                        </p>
                    </div>
                    <div className="flex flex-col gap-0.5 p-2">
                        {groupJobs.map((job) => (
                            <HistoryRow key={job.id} job={job} />
                        ))}
                    </div>
                </div>
            ))}
        </div>
    );
}
