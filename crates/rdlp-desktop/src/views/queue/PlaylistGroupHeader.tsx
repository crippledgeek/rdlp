// PlaylistGroupHeader: collapsible header for a playlist group in the queue.
// Shows playlist title, aggregate progress bar, status counts, expand/collapse toggle.

import { useMemo } from "react";
import { ChevronDown, ChevronRight, XCircle } from "lucide-react";
import { cancelDownload } from "@/api/downloads";
import { isInFlight, isTerminal, tallyByStatus } from "@/lib/jobStatus";
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
        const tally = tallyByStatus(jobs);
        const total = jobs.length;
        const progressFraction = total > 0 ? tally.completed / total : 0;
        return { ...tally, total, progressFraction };
    }, [jobs]);

    const hasActive = stats.running > 0 || stats.pending > 0 || stats.processing > 0;

    async function handleCancelAll() {
        const activeJobs = jobs.filter((j) => !isTerminal(j.status));
        for (const job of activeJobs) {
            if (isInFlight(job.status)) {
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
                    <ChevronRight className="w-3.5 h-3.5 text-[var(--text-muted)] shrink-0" />
                ) : (
                    <ChevronDown className="w-3.5 h-3.5 text-[var(--text-muted)] shrink-0" />
                )}

                <span className="flex-1 min-w-0 text-[13px] font-medium text-[#F8FAFC] truncate">
                    {playlistTitle}
                </span>

                <span className="text-[11px] text-[var(--text-muted)] tabular-nums shrink-0" aria-live="polite">
                    {stats.completed}/{stats.total} completed
                </span>

                {hasActive && (
                    <button
                        onClick={(e) => {
                            e.stopPropagation();
                            void handleCancelAll();
                        }}
                        aria-label="Cancel all downloads in this playlist"
                        className="p-1 rounded-[4px] text-[var(--text-muted)] hover:text-[#e85858] hover:bg-[#2a0a0a] transition-colors"
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

            {/* Active episode indicator */}
            {stats.running > 0 && (() => {
                const activeJob = jobs.find((j) => j.status === "running");
                return activeJob?.statusMessage ? (
                    <p className="text-[10px] text-[#aaaaaa] truncate" aria-live="polite">
                        {activeJob.statusMessage}
                    </p>
                ) : null;
            })()}

            {/* Status summary */}
            <div className="flex items-center gap-2 text-[10px]">
                {stats.running > 0 && (
                    <span className="text-[#4a9eff]">{stats.running} downloading</span>
                )}
                {stats.processing > 0 && (
                    <span className="text-[#a78bfa]">{stats.processing} processing</span>
                )}
                {stats.completed > 0 && (
                    <span className="text-[#22C55E]">{stats.completed} completed</span>
                )}
                {stats.pending > 0 && (
                    <span className="text-[var(--text-muted)]">{stats.pending} pending</span>
                )}
                {stats.failed > 0 && (
                    <span className="text-[#EF4444]">{stats.failed} failed</span>
                )}
                {stats.cancelled > 0 && (
                    <span className="text-[var(--text-muted)]">{stats.cancelled} cancelled</span>
                )}
            </div>
        </div>
    );
}
