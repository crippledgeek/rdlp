// Status badge for download job lifecycle states.

import { cn } from "@/lib/utils";
import type { JobStatus } from "@/types";

interface StatusBadgeProps {
    status: JobStatus;
    className?: string;
}

const statusConfig: Record<JobStatus, { label: string; className: string }> = {
    pending: {
        label: "Queued",
        className: "text-[#aaaaaa] bg-[#1a1a2e] border border-[#2a2a3e]",
    },
    running: {
        label: "Downloading",
        className: "text-[#e8a838] bg-[#2a1f0a] border border-[#3a2a0e]",
    },
    completed: {
        label: "Completed",
        className: "text-[#4a9e4a] bg-[#0a2010] border border-[#0e3014]",
    },
    failed: {
        label: "Failed",
        className: "text-[#e85858] bg-[#2a0a0a] border border-[#3a0e0e]",
    },
    cancelled: {
        label: "Cancelled",
        className: "text-[var(--text-muted)] bg-[#141414] border border-[#1e1e1e]",
    },
};

export function StatusBadge({ status, className }: StatusBadgeProps) {
    const config = statusConfig[status] ?? statusConfig.pending;
    // 'failed' is the only assertive transition; everything else is polite.
    const role = status === "failed" ? "alert" : "status";
    return (
        <span
            role={role}
            className={cn(
                "inline-flex items-center px-[6px] py-[2px] rounded-[3px] text-[10px] font-medium",
                config.className,
                className,
            )}
        >
            {config.label}
        </span>
    );
}
