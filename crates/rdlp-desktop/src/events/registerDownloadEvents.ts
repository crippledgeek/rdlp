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
    onDownloadLog,
    onFormatSelected,
    onPostProcessProgress,
} from "../lib/tauri";
import { queryKeys } from "../query/queryKeys";
import type { DownloadJob } from "../types";

interface ProgressEntry {
    progress: number;
    speed: string | null;
    eta: string | null;
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
                    return entry ? { ...job, ...entry } : job;
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
                                  progress: 1.0,
                                  output_path: p.filepath || null,
                                  completed_at: Math.floor(Date.now() / 1000),
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
        onDownloadLog((p) => {
            if (!mounted) return;
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? {
                                  ...job,
                                  statusMessage: p.message,
                                  logMessages: [...(job.logMessages || []), p.message],
                              }
                            : job,
                    ),
            );
        }),
    );

    unlisteners.push(
        onFormatSelected((p) => {
            if (!mounted) return;
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? { ...job, statusMessage: `Format: ${p.quality}` }
                            : job,
                    ),
            );
        }),
    );

    unlisteners.push(
        onPostProcessProgress((p) => {
            if (!mounted) return;
            const pct = Math.round(p.progress * 100);
            qc.setQueryData<DownloadJob[]>(
                queryKeys.downloads.list(),
                (old) =>
                    old?.map((job) =>
                        job.id === p.jobId
                            ? {
                                  ...job,
                                  statusMessage: `${p.stage}… ${pct}%`,
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
