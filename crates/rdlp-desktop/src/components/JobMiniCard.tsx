// Compact job progress card for NavPanel. Shows title, progress bar, percentage.
// Click switches to Queue view and selects the job.

import { setView, setSelectedJob } from "@/stores/uiStore";
import { isInFlight } from "@/lib/jobStatus";
import { cn, displayTitle, progressPercentLabel } from "@/lib/utils";
import type { DownloadJob } from "@/types";

interface JobMiniCardProps {
    job: DownloadJob;
    className?: string;
}

export function JobMiniCard({ job, className }: JobMiniCardProps) {
    const progress = job.progress ?? 0;
    const title = displayTitle(job.title);
    const inFlight = isInFlight(job.status);
    const isPending = job.status === "pending";

    function handleClick() {
        setView("queue");
        setSelectedJob(job.id);
    }

    return (
        <button
            onClick={handleClick}
            className={cn(
                "w-full text-left px-2 py-1.5 rounded-[4px] hover:bg-[#141428] transition-colors cursor-pointer",
                className,
            )}
        >
            <div className="flex items-center justify-between gap-2 mb-1">
                <span className="text-[11px] text-[#eeeeee] truncate flex-1 leading-tight">{title}</span>
                {inFlight && (
                    <span className="text-[10px] text-[#aaaaaa] shrink-0 font-mono">{progressPercentLabel(job.progress, job.status === "processing")}%</span>
                )}
                {isPending && (
                    <span className="text-[10px] text-[var(--text-muted)] shrink-0">Queued</span>
                )}
            </div>
            {inFlight && (
                <div className="h-[2px] rounded-full bg-[#1a1a2e] overflow-hidden">
                    <div
                        className="h-full bg-[#4a9eff] transition-[width] duration-300 rounded-full"
                        style={{ width: `${progress * 100}%` }}
                    />
                </div>
            )}
            {job.speed && (
                <span className="text-[10px] text-[var(--text-muted)] font-mono">{job.speed}</span>
            )}
        </button>
    );
}
