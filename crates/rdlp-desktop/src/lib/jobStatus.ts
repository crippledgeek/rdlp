// Shared job lifecycle state transitions for the downloads cache.

import type { DownloadJob, JobStatus } from "../types";

/**
 * The field patch applied when a job reaches the terminal `cancelled` state:
 * sets `status` and clears every transient progress field.
 *
 * Single source of truth used by BOTH cancel paths — the optimistic flip in
 * `api/downloads.cancelDownload` and the `download-cancelled` event reducer in
 * `events/registerDownloadEvents`. They MUST stay in lock-step; a field added
 * to one path but not the other would desync the optimistic and confirmed
 * cancel state. Spread over the existing job: `{ ...job, ...cancelledJobPatch() }`.
 */
export function cancelledJobPatch(): Partial<DownloadJob> {
    return {
        status: "cancelled",
        currentUnit: null,
        speed: null,
        eta: null,
        progress: null,
    };
}

/** The per-job sub-label, derived during render from canonical fields. Null = no sub-label. */
export function jobSubLabel(job: DownloadJob): string | null {
    if (job.status === "processing") return job.stage;
    const u = job.currentUnit;
    if (!u) return null;
    const pct = Math.round((job.progress ?? 0) * 100);
    if (u.total === 2 && (u.title === "Video" || u.title === "Audio")) {
        return `${u.title} — ${pct}%`;
    }
    return `Ep ${u.index}/${u.total} — ${u.title}`;
}

/** Categorizable buckets used by the Queue filter tabs + STATS panel. */
export type QueueBucket = "active" | "processing" | "completed" | "cancelled" | "failed";

/** Filter keys = the buckets plus the catch-all. Canonical definition. */
export type QueueFilter = "all" | QueueBucket;

/**
 * Single source of truth: which `JobStatus` values fall into each bucket.
 * The six statuses are partitioned across the five buckets with no overlap.
 */
const BUCKET_STATUSES: Record<QueueBucket, readonly JobStatus[]> = {
    active: ["running", "pending"],
    processing: ["processing"],
    completed: ["completed"],
    cancelled: ["cancelled"],
    failed: ["failed"],
};

/** Terminal states — used by "Clear Finished" and playlist auto-collapse. */
export const TERMINAL_STATUSES: readonly JobStatus[] = ["completed", "cancelled", "failed"];

/** The terminal buckets (mirror of TERMINAL_STATUSES in bucket space). */
export const TERMINAL_BUCKETS: readonly QueueBucket[] = ["completed", "cancelled", "failed"];

/** True when the job has reached a terminal state (completed, cancelled, or failed). */
export function isTerminal(status: JobStatus): boolean {
    return TERMINAL_STATUSES.includes(status);
}

/**
 * True when a job finished without failing — completed or cancelled.
 * Used for playlist auto-collapse: a group collapses once every item is
 * done-or-cancelled, but a failed item keeps it expanded for attention.
 */
export function isDoneNotFailed(status: JobStatus): boolean {
    return isTerminal(status) && status !== "failed";
}

/** Does a job's status match the given filter key? `"all"` matches every status. */
export function jobMatchesFilter(status: JobStatus, filter: QueueFilter): boolean {
    if (filter === "all") return true;
    return BUCKET_STATUSES[filter].includes(status);
}

/** Map a status to its single owning bucket. Derived from `BUCKET_STATUSES`. */
function bucketOf(status: JobStatus): QueueBucket {
    for (const bucket of Object.keys(BUCKET_STATUSES) as QueueBucket[]) {
        if (BUCKET_STATUSES[bucket].includes(status)) return bucket;
    }
    // Unreachable: JobStatus is exhaustively partitioned across the buckets above.
    throw new Error(`unclassified job status: ${status}`);
}

/** Per-filter counts (including `all`), derived from the job list. */
export function countByBucket(jobs: DownloadJob[]): Record<QueueFilter, number> {
    const counts: Record<QueueFilter, number> = {
        all: jobs.length,
        active: 0,
        processing: 0,
        completed: 0,
        cancelled: 0,
        failed: 0,
    };
    for (const job of jobs) counts[bucketOf(job.status)]++;
    return counts;
}

/** Per-status tally across all six `JobStatus` values, derived from the job list. */
export function tallyByStatus(jobs: DownloadJob[]): Record<JobStatus, number> {
    const tally: Record<JobStatus, number> = {
        pending: 0,
        running: 0,
        processing: 0,
        completed: 0,
        failed: 0,
        cancelled: 0,
    };
    for (const job of jobs) tally[job.status]++;
    return tally;
}

/** In-flight: actively downloading OR post-processing — shows progress, is cancellable. */
export function isInFlight(status: JobStatus): boolean {
    return status === "running" || status === "processing";
}
