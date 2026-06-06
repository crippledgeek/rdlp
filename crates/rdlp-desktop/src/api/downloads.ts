// TanStack Query options and mutations for download-related Rust commands.

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import { queryClient } from "../query/queryClient";
import { cancelledJobPatch } from "../lib/jobStatus";
import type { DownloadJob, DownloadOptions, PlaylistContext } from "../types";

/** Fetch the current download queue. */
export function downloadsQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.downloads.list(),
        queryFn: () => invokeTyped<DownloadJob[]>("queue"),
    });
}

/** Start a download. Returns the job ID from the backend. */
export async function startDownload(
    url: string,
    options: DownloadOptions,
    title?: string,
    playlistContext?: PlaylistContext,
): Promise<string> {
    const jobId = await invokeTyped<string>("start_download", {
        url,
        options,
        title: title ?? null,
        playlistContext: playlistContext ?? null,
    });
    await queryClient.invalidateQueries({ queryKey: queryKeys.downloads.list() });
    return jobId;
}

/** Cancel a running download. Optimistically flips to cancelled; rolls back on IPC failure. */
export async function cancelDownload(jobId: string): Promise<void> {
    const key = queryKeys.downloads.list();
    // Guard the optimistic write against an in-flight refetch (TanStack optimistic-updates pattern).
    await queryClient.cancelQueries({ queryKey: key });
    const previous = queryClient.getQueryData<DownloadJob[]>(key);
    queryClient.setQueryData<DownloadJob[]>(key, (old) =>
        old?.map((job) => (job.id === jobId ? { ...job, ...cancelledJobPatch() } : job)),
    );
    try {
        await invokeTyped<void>("cancel_download", { jobId });
    } catch (e) {
        if (previous) queryClient.setQueryData(key, previous); // rollback
        throw e;
    }
}

/** Remove a completed/failed/cancelled job from the queue. */
export async function removeJob(jobId: string): Promise<void> {
    await invokeTyped<void>("remove_job", { jobId });
    await queryClient.invalidateQueries({ queryKey: queryKeys.downloads.list() });
}

/** Remove all completed/failed/cancelled jobs from the queue. */
export async function clearCompletedJobs(): Promise<void> {
    await invokeTyped<void>("clear_completed_jobs");
    await queryClient.invalidateQueries({ queryKey: queryKeys.downloads.list() });
}
