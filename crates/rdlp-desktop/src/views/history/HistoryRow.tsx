// HistoryRow: single completed download entry in the history list.

import { useStore } from "@tanstack/react-store";
import { FolderOpen, RotateCcw } from "lucide-react";
import { uiStore, setSelectedJob, setView } from "@/stores/uiStore";
import { startDownload } from "@/api/downloads";
import { StatusBadge } from "@/components/StatusBadge";
import { Thumbnail } from "@/components/Thumbnail";
import { invokeTyped } from "@/api/invokeClient";
import { cn } from "@/lib/utils";
import type { DownloadJob } from "@/types";

interface HistoryRowProps {
    job: DownloadJob;
}

function formatRelativeDate(ts: number | null): string {
    if (!ts) return "";
    const diff = Date.now() - ts * 1000;
    const minutes = Math.floor(diff / 60_000);
    const hours = Math.floor(diff / 3_600_000);
    const days = Math.floor(diff / 86_400_000);
    if (minutes < 1) return "just now";
    if (minutes < 60) return `${minutes}m ago`;
    if (hours < 24) return `${hours}h ago`;
    return `${days}d ago`;
}

export function HistoryRow({ job }: HistoryRowProps) {
    const selectedJobId = useStore(uiStore, (s) => s.selectedJobId);
    const isSelected = selectedJobId === job.id;

    const handleReveal = (e: React.MouseEvent) => {
        e.stopPropagation();
        if (!job.output_path) return;
        invokeTyped<void>("reveal_in_folder", { path: job.output_path }).catch(console.error);
    };

    const handleRedownload = (e: React.MouseEvent) => {
        e.stopPropagation();
        if (!job.options) return;
        startDownload(job.url, job.options, job.title ?? undefined)
            .then(() => { setView("queue"); })
            .catch(console.error);
    };

    return (
        <div
            onClick={() => setSelectedJob(job.id)}
            className={cn(
                "flex items-center gap-3 p-3 rounded-[6px] cursor-pointer transition-colors border",
                isSelected
                    ? "bg-[#0f1a2e] border-[#2a3a5a]"
                    : "bg-[var(--surface-elevated)] border-[#1a1a2e] hover:border-[#2a2a3e]",
            )}
        >
            {/* Thumbnail */}
            <div className="w-[56px] h-[36px] shrink-0 rounded-[3px] overflow-hidden bg-[#0e0e1e]">
                <Thumbnail
                    src={null}
                    alt={job.title ?? ""}
                    className="w-full h-full object-cover"
                />
            </div>

            {/* Content */}
            <div className="flex-1 min-w-0">
                <p className="text-[13px] font-medium text-[#eeeeee] truncate leading-snug">
                    {job.title ?? job.url}
                </p>
                <div className="flex items-center gap-2 mt-0.5">
                    {job.completed_at && (
                        <span className="text-[10px] text-[var(--text-muted)]">
                            {formatRelativeDate(job.completed_at)}
                        </span>
                    )}
                    {job.output_path && (
                        <span className="text-[10px] text-[var(--text-muted)] truncate">
                            {job.output_path.split(/[\\/]/).pop()}
                        </span>
                    )}
                </div>
            </div>

            {/* Status + actions */}
            <div className="flex items-center gap-1.5 shrink-0">
                <StatusBadge status={job.status} />
                {job.output_path && (
                    <button
                        onClick={handleReveal}
                        aria-label="Reveal in folder"
                        className="p-1.5 rounded-[4px] text-[var(--text-muted)] hover:text-[#4a9eff] hover:bg-[#0f1a2e] transition-colors"
                    >
                        <FolderOpen className="w-3 h-3" />
                    </button>
                )}
                {job.options && (
                    <button
                        onClick={handleRedownload}
                        aria-label="Re-download"
                        className="p-1.5 rounded-[4px] text-[var(--text-muted)] hover:text-[#4a9eff] hover:bg-[#0f1a2e] transition-colors"
                    >
                        <RotateCcw className="w-3 h-3" />
                    </button>
                )}
            </div>
        </div>
    );
}
