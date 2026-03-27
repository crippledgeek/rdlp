// PlaylistGroupHeader: collapsible header for a playlist group in the queue.
// Shows playlist title, aggregate progress bar, status counts, expand/collapse toggle.

import { useMemo } from "react";
import { ChevronDown, ChevronRight, XCircle } from "lucide-react";
import { cancelDownload } from "@/api/downloads";
import type { DownloadJob } from "@/types";

interface PlaylistGroupHeaderProps {
    playlistId: string;
    playlistTitle: string;
    jobs: DownloadJob[];
    collapsed: boolean;
    onToggleCollapse: (playlistId: string) => void;
}

export function PlaylistGroupHeader({
    playlistId,
    playlistTitle,
    jobs,
    collapsed,
    onToggleCollapse,
}: PlaylistGroupHeaderProps) {
    const stats = useMemo(() => {
        let completed = 0;
        let running = 0;
        let pending = 0;
        let failed = 0;
        let cancelled = 0;

        for (const job of jobs) {
            switch (job.status) {
                case "completed":
                    completed++;
                    break;
                case "running":
                    running++;
                    break;
                case "pending":
                    pending++;
                    break;
                case "failed":
                    failed++;
                    break;
                case "cancelled":
                    cancelled++;
                    break;
            }
        }

        const total = jobs.length;
        const progressFraction = total > 0 ? completed / total : 0;

        return { completed, running, pending, failed, cancelled, total, progressFraction };
    }, [jobs]);

    const hasActive = stats.running > 0 || stats.pending > 0;

    async function handleCancelAll() {
        const activeJobs = jobs.filter((j) => j.status === "running" || j.status === "pending");
        for (const job of activeJobs) {
            if (job.status === "running") {
                void cancelDownload(job.id);
            }
        }
    }

    return (
        <div
            className="flex flex-col gap-1 px-3 py-1.5 bg-[#1E293B] rounded-t-[6px] cursor-pointer select-none"
            onClick={() => onToggleCollapse(playlistId)}
            role="button"
            aria-expanded={!collapsed}
            aria-label={`${playlistTitle} playlist group, ${stats.completed} of ${stats.total} completed`}
        >
            {/* Title row */}
            <div className="flex items-center gap-2">
                {collapsed ? (
                    <ChevronRight className="w-3.5 h-3.5 text-[#666666] shrink-0" />
                ) : (
                    <ChevronDown className="w-3.5 h-3.5 text-[#666666] shrink-0" />
                )}

                <span className="flex-1 min-w-0 text-[13px] font-medium text-[#F8FAFC] truncate">
                    {playlistTitle}
                </span>

                <span className="text-[11px] text-[#888888] tabular-nums shrink-0">
                    {stats.completed}/{stats.total} completed
                </span>

                {hasActive && (
                    <button
                        onClick={(e) => {
                            e.stopPropagation();
                            void handleCancelAll();
                        }}
                        aria-label="Cancel all downloads in this playlist"
                        className="p-1 rounded-[4px] text-[#666666] hover:text-[#e85858] hover:bg-[#2a0a0a] transition-colors"
                    >
                        <XCircle className="w-3.5 h-3.5" />
                    </button>
                )}
            </div>

            {/* Aggregate progress bar */}
            <div className="h-[2px] rounded-full bg-[#0F172A] overflow-hidden">
                <div
                    className="h-full bg-[#4a9eff] transition-[width] duration-300 rounded-full"
                    style={{ width: `${Math.round(stats.progressFraction * 100)}%` }}
                />
            </div>

            {/* Status summary */}
            <div className="flex items-center gap-2 text-[10px]">
                {stats.running > 0 && (
                    <span className="text-[#4a9eff]">{stats.running} downloading</span>
                )}
                {stats.completed > 0 && (
                    <span className="text-[#22C55E]">{stats.completed} completed</span>
                )}
                {stats.pending > 0 && (
                    <span className="text-[#666666]">{stats.pending} pending</span>
                )}
                {stats.failed > 0 && (
                    <span className="text-[#EF4444]">{stats.failed} failed</span>
                )}
                {stats.cancelled > 0 && (
                    <span className="text-[#888888]">{stats.cancelled} cancelled</span>
                )}
            </div>
        </div>
    );
}
