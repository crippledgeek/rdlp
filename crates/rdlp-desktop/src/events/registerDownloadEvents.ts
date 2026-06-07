// Single event wiring point for all Tauri download events.
//
// Called once from App root useEffect. Updates the TanStack Query cache
// directly via setQueryData — no intermediate stores.
//
// Progress events are batched per-animation-frame to avoid excess
// re-renders when multiple downloads are active simultaneously.

import type { QueryClient } from "@tanstack/react-query";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
    onDownloadProgress,
    onDownloadComplete,
    onDownloadError,
    onDownloadCancelled,
    onDownloadLog,
    onPostProcessProgress,
    onUnitStarted,
} from "../lib/tauri";
import { queryKeys } from "../query/queryKeys";
import { appendLog } from "../components/LogViewer";
import { cancelledJobPatch } from "../lib/jobStatus";
import type { DownloadJob } from "../types";

interface ProgressEntry {
    progress: number;
    speed: string | null;
    eta: string | null;
    totalBytes: number | null;
    isEstimated: boolean;
    segmentsDownloaded: number | null;
    totalSegments: number | null;
}

/**
 * Register all Tauri download event listeners.
 *
 * Returns a cleanup function that unregisters everything, cancels
 * the rAF timer, and clears the pending progress buffer.
 */
export function registerDownloadEvents(qc: QueryClient): () => void {
    const unlisteners: Promise<UnlistenFn>[] = [];
    let mounted = true;

    // --- rAF-batched progress updates ---
    const pending = new Map<string, ProgressEntry>();
    let rafId: number | null = null;

    function flush() {
        rafId = null;
        if (!mounted || pending.size === 0) return;

        const batch = new Map(pending);
        pending.clear();

        qc.setQueryData<DownloadJob[]>(
            queryKeys.downloads.list(),
            (old) => {
                if (!old) return old;
                return old.map((job) => {
                    const entry = batch.get(job.id);
                    if (!entry) return job;
                    return { ...job, ...entry };
                });
            },
        );
    }

    function scheduleFlush() {
        if (rafId === null) {
            rafId = requestAnimationFrame(flush);
        }
    }

    // --- Event listeners ---

    unlisteners.push(
        onDownloadProgress((p) => {
            if (!mounted) return;
            pending.set(p.jobId, {
                progress: p.progress,
                speed: p.speed,
                eta: p.eta,
                totalBytes: p.totalBytes,
                isEstimated: p.isEstimated,
                segmentsDownloaded: p.segmentsDownloaded,
                totalSegments: p.totalSegments,
            });
            scheduleFlush();
        }),
    );

    unlisteners.push(
        onDownloadComplete((p) => {
            if (!mounted) return;
            pending.delete(p.jobId);
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? {
                                  ...job,
                                  status: "completed" as const,
                                  progress: 1,
                                  output_path: p.filepath || null,
                                  completed_at: Math.floor(Date.now() / 1000),
                                  currentUnit: null,
                              }
                            : job,
                    ),
            );
        }),
    );

    unlisteners.push(
        onDownloadError((p) => {
            if (!mounted) return;
            pending.delete(p.jobId);
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? {
                                  ...job,
                                  status: "failed" as const,
                                  error: p.error,
                                  retryable: p.retryable,
                              }
                            : job,
                    ),
            );
        }),
    );

    unlisteners.push(
        onDownloadCancelled((p) => {
            if (!mounted) return;
            pending.delete(p.jobId);
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? { ...job, ...cancelledJobPatch() }
                            : job,
                    ),
            );
        }),
    );

    unlisteners.push(
        onDownloadLog((p) => {
            if (!mounted) return;
            // Forward to global log viewer ring buffer
            const level = p.level === "warn" ? "warn" : p.level === "debug" ? "debug" : "info";
            appendLog(level, p.message, p.jobId);
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? {
                                  ...job,
                                  logMessages: [...(job.logMessages || []), p.message],
                              }
                            : job,
                    ),
            );
        }),
    );

    unlisteners.push(
        onPostProcessProgress((p) => {
            if (!mounted) return;
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? {
                                  ...job,
                                  progress: p.progress,
                                  speed: null,
                                  eta: p.eta,
                              }
                            : job,
                    ),
            );
        }),
    );

    unlisteners.push(
        onUnitStarted((p) => {
            if (!mounted) return;
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? {
                                  ...job,
                                  // Live-push mirror of the Rust-owned `current_unit`; the queue refetch reconciles to Rust's value.
                                  currentUnit: {
                                      index: p.unitIndex,
                                      total: p.unitTotal,
                                      title: p.unitTitle,
                                  },
                              }
                            : job,
                    ),
            );
        }),
    );

    // --- Cleanup ---
    return () => {
        mounted = false;
        if (rafId !== null) cancelAnimationFrame(rafId);
        pending.clear();
        for (const p of unlisteners) {
            p.then((unlisten) => unlisten());
        }
    };
}
