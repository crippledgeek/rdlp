// TanStack Query options and mutations for download-related Rust commands.

import { queryOptions } from "@tanstack/react-query";
import { invokeTyped } from "./invokeClient";
import { queryKeys } from "../query/queryKeys";
import { queryClient } from "../query/queryClient";
import type { AppSettings, DownloadJob, DownloadOptions } from "../types";

/** Fetch the current download queue. */
export function downloadsQueryOptions() {
    return queryOptions({
        queryKey: queryKeys.downloads.list(),
        queryFn: () => invokeTyped<DownloadJob[]>("get_queue"),
    });
}

/** Build default DownloadOptions from AppSettings. */
export function buildDefaultOptions(settings: AppSettings | null): DownloadOptions {
    return {
        format: null,
        outputDir: null,
        subtitles: (settings?.default_subtitle_langs ?? []).length > 0,
        subtitleLangs: settings?.default_subtitle_langs ?? [],
        remux: settings?.default_remux ?? null,
        extractAudio: settings?.default_extract_audio ?? null,
        embedThumbnail: settings?.embed_thumbnail ?? true,
        audioMultistreams: false,
        recodeVideo: null,
    };
}

/** Start a download. Returns the job ID from the backend. */
export async function startDownload(
    url: string,
    options: DownloadOptions,
): Promise<string> {
    const jobId = await invokeTyped<string>("start_download", { url, options });
    await queryClient.invalidateQueries({ queryKey: queryKeys.downloads.list() });
    return jobId;
}

/** Cancel a running download. */
export async function cancelDownload(jobId: string): Promise<void> {
    await invokeTyped<void>("cancel_download", { jobId });
    await queryClient.invalidateQueries({ queryKey: queryKeys.downloads.list() });
}

/** Remove a completed/failed/cancelled job from the queue. */
export async function removeJob(jobId: string): Promise<void> {
    await invokeTyped<void>("remove_job", { jobId });
    await queryClient.invalidateQueries({ queryKey: queryKeys.downloads.list() });
}
