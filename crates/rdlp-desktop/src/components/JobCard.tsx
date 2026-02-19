import type { DownloadJob } from "../types";

interface JobCardProps {
    job: DownloadJob;
    onCancel: (id: string) => void;
    onRemove: (id: string) => void;
    onRetry: (url: string) => void;
}

/** Format a progress fraction (0..1 or 0..100) as a percentage string. */
function formatPercent(progress: number | null): string {
    if (progress === null) return "0%";
    // Normalise: values > 1 are already percentages, values <= 1 are fractions.
    const pct = progress > 1 ? progress : progress * 100;
    return `${pct.toFixed(1)}%`;
}

/** Compute the CSS width for the progress bar fill. */
function progressWidth(progress: number | null): string {
    if (progress === null) return "0%";
    const pct = progress > 1 ? progress : progress * 100;
    return `${Math.min(pct, 100)}%`;
}

/** Displays a single download job with status, progress, and action buttons. */
export function JobCard({ job, onCancel, onRemove, onRetry }: JobCardProps) {
    const displayTitle = job.title ?? job.url;
    const isRunning = job.status === "running";
    const isFailed = job.status === "failed";
    const isTerminal =
        job.status === "completed" ||
        job.status === "failed" ||
        job.status === "cancelled";

    return (
        <div className="job-card">
            <div className="job-header">
                <span className="job-title" title={job.url}>
                    {displayTitle}
                </span>
                <span className={`job-status status-${job.status}`}>
                    {job.status}
                </span>
            </div>

            {isRunning && (
                <div className="job-progress-section">
                    <div className="job-progress-bar">
                        <div
                            className="job-progress-fill"
                            style={{ width: progressWidth(job.progress) }}
                        />
                    </div>
                    <div className="job-progress-details">
                        <span className="job-percent">
                            {formatPercent(job.progress)}
                        </span>
                        {job.speed && (
                            <span className="job-speed">{job.speed}</span>
                        )}
                        {job.eta && (
                            <span className="job-eta">ETA: {job.eta}</span>
                        )}
                    </div>
                </div>
            )}

            {isFailed && job.error && (
                <div className="job-error">{job.error}</div>
            )}

            <div className="job-actions">
                {isRunning && (
                    <button
                        className="job-action-button job-cancel-button"
                        onClick={() => onCancel(job.id)}
                    >
                        Cancel
                    </button>
                )}
                {isFailed && job.retryable && (
                    <button
                        className="job-action-button job-retry-button"
                        onClick={() => onRetry(job.url)}
                    >
                        Retry
                    </button>
                )}
                {isTerminal && (
                    <button
                        className="job-action-button job-remove-button"
                        onClick={() => onRemove(job.id)}
                    >
                        Remove
                    </button>
                )}
            </div>
        </div>
    );
}
