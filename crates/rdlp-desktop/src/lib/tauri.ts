// Typed wrappers around Tauri's invoke() and listen() IPC functions.
//
// Each function mirrors a #[tauri::command] on the Rust backend.
// Event listeners mirror the Tauri events emitted from src-tauri/src/events.rs.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
    AppSettings,
    DownloadCancelledPayload,
    DownloadCompletePayload,
    DownloadErrorPayload,
    DownloadJob,
    DownloadLogPayload,
    DownloadOptions,
    DownloadProgressPayload,
    FormatListResponse,
    PostProcessProgressPayload,
    SearchFilter,
    SearchFilterDescriptor,
    SearchPageResponse,
    SearchSiteInfo,
    UnitStartedPayload,
} from "../types";

// ========== Search ==========

/** Search a supported site by query string. */
export async function searchContent(
    query: string,
    site: string,
    filters: SearchFilter[] = [],
    page?: number,
): Promise<SearchPageResponse> {
    return invoke<SearchPageResponse>("search_content", { query, site, filters, page });
}

/** List all sites that support search. */
export async function getSearchProviders(): Promise<SearchSiteInfo[]> {
    return invoke<SearchSiteInfo[]>("search_providers");
}

/** Retrieve available search filters for a given site. */
export async function getSearchFilters(
    site: string,
): Promise<SearchFilterDescriptor[]> {
    return invoke<SearchFilterDescriptor[]>("search_filters", { site });
}

// ========== Formats ==========

/** Retrieve available formats for a URL. */
export async function getFormats(url: string): Promise<FormatListResponse> {
    return invoke<FormatListResponse>("formats", { url });
}

// ========== Download ==========

/** Start a new download for the given URL. Returns the job UUID. */
export async function startDownload(
    url: string,
    options: DownloadOptions,
    title?: string,
): Promise<string> {
    return invoke<string>("start_download", { url, options, title: title ?? null });
}

/** Cancel an in-progress download by its job ID. */
export async function cancelDownload(jobId: string): Promise<void> {
    return invoke<void>("cancel_download", { jobId });
}

/** Retrieve the current download queue. */
export async function getQueue(): Promise<DownloadJob[]> {
    return invoke<DownloadJob[]>("queue");
}

/** Remove a completed, failed, or cancelled download from the queue. */
export async function removeJob(jobId: string): Promise<void> {
    return invoke<void>("remove_job", { jobId });
}

// ========== Settings ==========

/** Retrieve the current application settings. */
export async function getSettings(): Promise<AppSettings> {
    return invoke<AppSettings>("settings");
}

/** Update application settings with new values. */
export async function updateSettings(settings: AppSettings): Promise<void> {
    return invoke<void>("update_settings", { settings });
}

/** Open a native directory picker dialog and return the selected path. */
export async function pickDirectory(): Promise<string | null> {
    return invoke<string | null>("pick_directory");
}

// ========== Event Listeners ==========

/** Subscribe to download progress events. Returns an unlisten function. */
export function onDownloadProgress(
    callback: (payload: DownloadProgressPayload) => void,
): Promise<UnlistenFn> {
    return listen<DownloadProgressPayload>("download-progress", (event) =>
        callback(event.payload),
    );
}

/** Subscribe to download completion events. Returns an unlisten function. */
export function onDownloadComplete(
    callback: (payload: DownloadCompletePayload) => void,
): Promise<UnlistenFn> {
    return listen<DownloadCompletePayload>("download-complete", (event) =>
        callback(event.payload),
    );
}

/** Subscribe to download error events. Returns an unlisten function. */
export function onDownloadError(
    callback: (payload: DownloadErrorPayload) => void,
): Promise<UnlistenFn> {
    return listen<DownloadErrorPayload>("download-error", (event) =>
        callback(event.payload),
    );
}

/** Subscribe to download cancellation events. Returns an unlisten function. */
export function onDownloadCancelled(
    callback: (payload: DownloadCancelledPayload) => void,
): Promise<UnlistenFn> {
    return listen<DownloadCancelledPayload>("download-cancelled", (event) =>
        callback(event.payload),
    );
}

/** Subscribe to download log events. Returns an unlisten function. */
export function onDownloadLog(
    callback: (payload: DownloadLogPayload) => void,
): Promise<UnlistenFn> {
    return listen<DownloadLogPayload>("download-log", (event) =>
        callback(event.payload),
    );
}

/** Subscribe to post-processing progress events. Returns an unlisten function. */
export function onPostProcessProgress(
    callback: (payload: PostProcessProgressPayload) => void,
): Promise<UnlistenFn> {
    return listen<PostProcessProgressPayload>("postprocess-progress", (event) =>
        callback(event.payload),
    );
}

/** Subscribe to unit-started events (playlist episode or merge stream start). Returns an unlisten function. */
export function onUnitStarted(
    handler: (payload: UnitStartedPayload) => void,
): Promise<UnlistenFn> {
    return listen<UnitStartedPayload>("unit-started", (e) => handler(e.payload));
}

/** Validate a format expression and return matching format IDs. */
export async function validateFormatExpression(
    expression: string,
    formats: FormatData[],
): Promise<string[]> {
    return invoke<string[]>("validate_format_expression", {
        expression,
        formats,
    });
}

/**
 * Format metadata for expression validation.
 *
 * Mirrors the Rust `FormatData` struct. Sent to the backend so
 * format filter predicates (e.g. `[height<=1080]`) can match.
 */
export interface FormatData {
    format_id: string;
    ext: string;
    width: number | null;
    height: number | null;
    fps: number | null;
    tbr: number | null;
    vcodec: string | null;
    acodec: string | null;
    filesize: number | null;
    vbr: number | null;
    abr: number | null;
    asr: number | null;
    protocol: string;
}
