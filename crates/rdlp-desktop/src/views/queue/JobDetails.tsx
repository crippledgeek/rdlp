// JobDetails: config panel content for selected job.
// Shows full job info, original options snapshot, logs, and action buttons.

import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { FolderOpen, RotateCcw, X } from "lucide-react";
import { uiStore, setSelectedJob } from "@/stores/uiStore";
import { downloadsQueryOptions, cancelDownload, removeJob, startDownload } from "@/api/downloads";
import { StatusBadge } from "@/components/StatusBadge";
import { isTerminal as isTerminalStatus, isInFlight } from "@/lib/jobStatus";
import { invokeTyped } from "@/api/invokeClient";
import { cn, progressPercent } from "@/lib/utils";

function formatDate(ts: number | null): string {
    if (!ts) return "—";
    return new Date(ts * 1000).toLocaleString();
}

export function JobDetails() {
    const selectedJobId = useStore(uiStore, (s) => s.selectedJobId);
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    if (!selectedJobId) return null;
    const job = jobs.find((j) => j.id === selectedJobId);
    if (!job) {
        return (
            <div className="p-3">
                <p className="text-[11px] text-[var(--text-muted)]">Job not found</p>
            </div>
        );
    }

    const isFailed = job.status === "failed";
    const isCompleted = job.status === "completed";
    const inFlight = isInFlight(job.status);
    const isTerminal = isTerminalStatus(job.status);

    const handleCancel = () => { cancelDownload(job.id).catch(console.error); };
    const handleRemove = () => {
        removeJob(job.id).catch(console.error);
        setSelectedJob(null);
    };
    const handleRetry = () => {
        if (!job.options) return;
        startDownload(job.url, job.options, job.title ?? undefined).catch(console.error);
    };
    const handleReveal = () => {
        if (!job.output_path) return;
        invokeTyped<void>("reveal_in_folder", { path: job.output_path }).catch(console.error);
    };

    return (
        <div className="flex flex-col gap-3 p-3 h-full overflow-y-auto">
            {/* Title + status */}
            <div>
                <div className="flex items-start gap-2 mb-1">
                    <p className="flex-1 min-w-0 text-[13px] font-medium text-[#eeeeee] leading-snug break-words">
                        {job.title ?? job.url}
                    </p>
                    <StatusBadge status={job.status} className="shrink-0 mt-0.5" />
                </div>
                <p className="text-[10px] text-[var(--text-muted)] break-all">{job.url}</p>
            </div>

            {/* Progress (running) */}
            {inFlight && (
                <div>
                    <div className="flex items-center justify-between text-[10px] text-[var(--text-muted)] mb-1">
                        <span className="font-mono text-[#aaaaaa]">{progressPercent(job.progress)}%</span>
                        <div className="flex items-center gap-2">
                            {job.speed && <span>{job.speed}</span>}
                            {job.eta && <span>ETA {job.eta}</span>}
                        </div>
                    </div>
                    <div className="h-[3px] rounded-full bg-[#1a1a2e] overflow-hidden">
                        <div
                            className="h-full bg-[#4a9eff] transition-[width] duration-300 rounded-full"
                            style={{ width: `${(job.progress ?? 0) * 100}%` }}
                        />
                    </div>
                    {job.statusMessage && (
                        <p className="text-[10px] text-[#4a9eff] mt-1 truncate">{job.statusMessage}</p>
                    )}
                </div>
            )}

            {/* Error */}
            {isFailed && job.error && (
                <div className="p-2 rounded-[4px] bg-[#2a0a0a] border border-[#3a0e0e]">
                    <p className="text-[11px] text-[#e85858]">{job.error}</p>
                </div>
            )}

            {/* Meta info */}
            <div className="space-y-1.5">
                <h3 className="section-heading">Info</h3>
                <div className="space-y-1">
                    {job.started_at && (
                        <div className="flex justify-between text-[11px]">
                            <span className="text-[var(--text-muted)]">Started</span>
                            <span className="text-[#aaaaaa] font-mono">{formatDate(job.started_at)}</span>
                        </div>
                    )}
                    {job.completed_at && (
                        <div className="flex justify-between text-[11px]">
                            <span className="text-[var(--text-muted)]">Completed</span>
                            <span className="text-[#aaaaaa] font-mono">{formatDate(job.completed_at)}</span>
                        </div>
                    )}
                    {job.output_path && (
                        <div className="text-[11px]">
                            <span className="text-[var(--text-muted)] block">Output</span>
                            <span className="text-[#aaaaaa] font-mono text-[10px] break-all">{job.output_path}</span>
                        </div>
                    )}
                </div>
            </div>

            {/* Download options snapshot */}
            {job.options && (
                <div className="space-y-1.5">
                    <h3 className="section-heading">Options</h3>
                    <div className="space-y-1">
                        {job.options.format && (
                            <div className="flex justify-between text-[11px]">
                                <span className="text-[var(--text-muted)]">Format</span>
                                <span className="text-[#aaaaaa] font-mono">{job.options.format}</span>
                            </div>
                        )}
                        {job.options.remux && (
                            <div className="flex justify-between text-[11px]">
                                <span className="text-[var(--text-muted)]">Remux</span>
                                <span className="text-[#aaaaaa] font-mono uppercase">{job.options.remux}</span>
                            </div>
                        )}
                        {job.options.extractAudio && (
                            <div className="flex justify-between text-[11px]">
                                <span className="text-[var(--text-muted)]">Audio</span>
                                <span className="text-[#aaaaaa] font-mono uppercase">{job.options.extractAudio}</span>
                            </div>
                        )}
                        {job.options.recodeVideo && (
                            <div className="flex justify-between text-[11px]">
                                <span className="text-[var(--text-muted)]">Recode</span>
                                <span className="text-[#aaaaaa] font-mono uppercase">{job.options.recodeVideo}</span>
                            </div>
                        )}
                        {job.options.videoEncoder && (
                            <div className="flex justify-between text-[11px]">
                                <span className="text-[var(--text-muted)]">Encoder</span>
                                <span className="text-[#aaaaaa] font-mono">{job.options.videoEncoder}</span>
                            </div>
                        )}
                        {job.options.recodeAudio && (
                            <div className="flex justify-between text-[11px]">
                                <span className="text-[var(--text-muted)]">Audio</span>
                                <span className="text-[#aaaaaa] font-mono">
                                    {job.options.recodeAudio.mode === "encoder"
                                        ? job.options.recodeAudio.name
                                        : job.options.recodeAudio.mode}
                                </span>
                            </div>
                        )}
                        {job.options.normalizeAudio && (
                            <div className="flex justify-between text-[11px]">
                                <span className="text-[var(--text-muted)]">Normalize</span>
                                <span className="text-[#4a9e4a] font-mono">on</span>
                            </div>
                        )}
                    </div>
                </div>
            )}

            {/* Log messages */}
            {job.logMessages && job.logMessages.length > 0 && (
                <div className="space-y-1.5">
                    <h3 className="section-heading">Logs ({job.logMessages.length})</h3>
                    <div className="max-h-[160px] overflow-y-auto space-y-0.5">
                        {job.logMessages.map((msg, i) => (
                            <p key={i} className="text-[10px] font-mono text-[var(--text-muted)] leading-relaxed">
                                {msg}
                            </p>
                        ))}
                    </div>
                </div>
            )}

            {/* Action buttons */}
            <div className={cn("flex flex-col gap-1.5 mt-auto pt-2 border-t border-[#1a1a2e]")}>
                {isCompleted && job.output_path && (
                    <button
                        onClick={handleReveal}
                        className="flex items-center gap-2 px-3 py-1.5 rounded-[4px] text-[12px] bg-[#0f1a2e] text-[#4a9eff] hover:bg-[#1a2a4a] transition-colors"
                    >
                        <FolderOpen className="w-3.5 h-3.5" />
                        Reveal in Folder
                    </button>
                )}
                {isFailed && job.options && (
                    <button
                        onClick={handleRetry}
                        className="flex items-center gap-2 px-3 py-1.5 rounded-[4px] text-[12px] bg-[#0f1a2e] text-[#4a9eff] hover:bg-[#1a2a4a] transition-colors"
                    >
                        <RotateCcw className="w-3.5 h-3.5" />
                        Retry
                    </button>
                )}
                {inFlight && (
                    <button
                        onClick={handleCancel}
                        className="flex items-center gap-2 px-3 py-1.5 rounded-[4px] text-[12px] text-[var(--text-muted)] hover:text-[#e85858] hover:bg-[#2a0a0a] transition-colors"
                    >
                        <X className="w-3.5 h-3.5" />
                        Cancel
                    </button>
                )}
                {isTerminal && (
                    <button
                        onClick={handleRemove}
                        className="flex items-center gap-2 px-3 py-1.5 rounded-[4px] text-[12px] text-[var(--text-muted)] hover:text-[var(--text-muted)] hover:bg-[#1a1a2e] transition-colors"
                    >
                        <X className="w-3.5 h-3.5" />
                        Remove
                    </button>
                )}
            </div>
        </div>
    );
}
