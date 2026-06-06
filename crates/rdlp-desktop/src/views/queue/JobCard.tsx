// JobCard: single job in the queue list.
// Shows thumbnail, title, status badge, progress bar, speed/ETA, action buttons.
// Compact mode (72px) used within playlist groups: no URL, episode title prominent.

import { useStore } from "@tanstack/react-store";
import { X, RotateCcw, FolderOpen } from "lucide-react";
import { uiStore, setSelectedJob } from "@/stores/uiStore";
import { cancelDownload, removeJob, startDownload } from "@/api/downloads";
import { StatusBadge } from "@/components/StatusBadge";
import { invokeTyped } from "@/api/invokeClient";
import { cn, displayTitle, progressPercent } from "@/lib/utils";
import type { DownloadJob } from "@/types";

interface JobCardProps {
    job: DownloadJob;
    /** Compact mode for playlist group members: 72px height, no URL, left border accent. */
    compact?: boolean;
}

export function JobCard({ job, compact = false }: JobCardProps) {
    const selectedJobId = useStore(uiStore, (s) => s.selectedJobId);
    const isSelected = selectedJobId === job.id;
    const isRunning = job.status === "running";
    const isFailed = job.status === "failed";
    const isCompleted = job.status === "completed";
    const isTerminal = isCompleted || isFailed || job.status === "cancelled";
    const progress = job.progress ?? 0;

    async function handleCancel() {
        await cancelDownload(job.id);
    }

    async function handleRemove() {
        await removeJob(job.id);
    }

    async function handleRetry() {
        if (!job.options) return;
        await startDownload(job.url, job.options, job.title ?? undefined);
    }

    async function handleReveal() {
        if (!job.output_path) return;
        await invokeTyped<void>("reveal_in_folder", { path: job.output_path });
    }

    // Episode label for compact mode (e.g. "Ep 3 — Episode Title")
    const compactTitle = compact && job.playlist
        ? `Ep ${job.playlist.playlistIndex} — ${displayTitle(job.title)}`
        : displayTitle(job.title);

    return (
        <div
            onClick={() => setSelectedJob(job.id)}
            className={cn(
                "flex gap-3 rounded-[6px] cursor-pointer transition-colors border",
                compact
                    ? "p-2 border-l-2 border-l-[#4a9eff]"
                    : "p-3",
                isSelected
                    ? "bg-[#0f1a2e] border-[#2a3a5a]"
                    : "bg-[var(--surface-elevated)] border-[#1a1a2e] hover:border-[#2a2a3e]",
                compact && !isSelected && "border-t-[#1a1a2e] border-r-[#1a1a2e] border-b-[#1a1a2e]",
            )}
        >
            {/* Main content */}
            <div className="flex-1 min-w-0 flex flex-col gap-1">
                {/* Title + status */}
                <div className="flex items-start gap-2">
                    <p className={cn(
                        "flex-1 min-w-0 font-medium text-[#eeeeee] truncate leading-snug",
                        compact ? "text-[12px]" : "text-[13px]",
                    )}>
                        {compactTitle}
                    </p>
                    <StatusBadge status={job.status} className="shrink-0 mt-0.5" />
                </div>

                {/* Progress bar */}
                {isRunning && (
                    <div>
                        <div className="h-[3px] rounded-full bg-[#1a1a2e] overflow-hidden mb-1">
                            <div
                                className="h-full bg-[#4a9eff] transition-[width] duration-300 rounded-full"
                                style={{ width: `${progress * 100}%` }}
                            />
                        </div>
                        <div className="flex items-center gap-3 text-[10px] text-[var(--text-muted)] font-mono">
                            <span className="text-[#aaaaaa]">{progressPercent(job.progress)}%</span>
                            {job.speed && <span>{job.speed}</span>}
                            {job.eta && <span>ETA {job.eta}</span>}
                            {job.statusMessage && (
                                <span className="text-[#4a9eff] truncate">{job.statusMessage}</span>
                            )}
                        </div>
                    </div>
                )}

                {/* Error message */}
                {isFailed && job.error && (
                    <p className="text-[11px] text-[#e85858] truncate">{job.error}</p>
                )}

                {/* Output path for completed (hidden in compact mode) */}
                {!compact && isCompleted && job.output_path && (
                    <p className="text-[10px] text-[var(--text-muted)] truncate">{job.output_path}</p>
                )}
            </div>

            {/* Action buttons */}
            <div className="flex items-center gap-1 shrink-0">
                {isRunning && (
                    <button
                        onClick={(e) => { e.stopPropagation(); void handleCancel(); }}
                        aria-label="Cancel download"
                        className="p-1.5 rounded-[4px] text-[var(--text-muted)] hover:text-[#e85858] hover:bg-[#2a0a0a] transition-colors"
                    >
                        <X className="w-3.5 h-3.5" />
                    </button>
                )}
                {isFailed && job.options && (
                    <button
                        onClick={(e) => { e.stopPropagation(); void handleRetry(); }}
                        aria-label="Retry download"
                        className="p-1.5 rounded-[4px] text-[var(--text-muted)] hover:text-[#4a9eff] hover:bg-[#0f1a2e] transition-colors"
                    >
                        <RotateCcw className="w-3.5 h-3.5" />
                    </button>
                )}
                {isCompleted && job.output_path && (
                    <button
                        onClick={(e) => { e.stopPropagation(); void handleReveal(); }}
                        aria-label="Reveal in folder"
                        className="p-1.5 rounded-[4px] text-[var(--text-muted)] hover:text-[#4a9eff] hover:bg-[#0f1a2e] transition-colors"
                    >
                        <FolderOpen className="w-3.5 h-3.5" />
                    </button>
                )}
                {isTerminal && (
                    <button
                        onClick={(e) => { e.stopPropagation(); void handleRemove(); }}
                        aria-label="Remove job"
                        className="p-1.5 rounded-[4px] text-[var(--text-muted)] hover:text-[var(--text-muted)] hover:bg-[#1a1a2e] transition-colors"
                    >
                        <X className="w-3 h-3" />
                    </button>
                )}
            </div>
        </div>
    );
}
