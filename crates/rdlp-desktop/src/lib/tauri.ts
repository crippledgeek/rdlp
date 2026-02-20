// Typed wrappers around Tauri's invoke() and listen() IPC functions.
//
// Each function mirrors a #[tauri::command] on the Rust backend.
// Event listeners mirror the Tauri events emitted from src-tauri/src/events.rs.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
    AppSettings,
    DownloadCompletePayload,
    DownloadErrorPayload,
    DownloadJob,
    DownloadLogPayload,
    DownloadOptions,
    DownloadProgressPayload,
    FormatListResponse,
    FormatSelectedPayload,
    SearchFilter,
    SearchFilterDescriptor,
    SearchResponse,
    SearchSiteInfo,
} from "../types";

// ========== Search ==========

/** Search a supported site by query string. */
export async function searchContent(
    query: string,
    site: string,
    filters: SearchFilter[] = [],
): Promise<SearchResponse> {
    return invoke<SearchResponse>("search_content", { query, site, filters });
}

/** List all sites that support search. */
export async function getSearchProviders(): Promise<SearchSiteInfo[]> {
    return invoke<SearchSiteInfo[]>("get_search_providers");
}

/** Retrieve available search filters for a given site. */
export async function getSearchFilters(
    site: string,
): Promise<SearchFilterDescriptor[]> {
    return invoke<SearchFilterDescriptor[]>("get_search_filters", { site });
}

// ========== Formats ==========

/** Retrieve available formats for a URL. */
export async function getFormats(url: string): Promise<FormatListResponse> {
    return invoke<FormatListResponse>("get_formats", { url });
}

// ========== Download ==========

/** Start a new download for the given URL. Returns the job UUID. */
export async function startDownload(
    url: string,
    options: DownloadOptions,
): Promise<string> {
    return invoke<string>("start_download", { url, options });
}

/** Cancel an in-progress download by its job ID. */
export async function cancelDownload(jobId: string): Promise<void> {
    return invoke<void>("cancel_download", { jobId });
}

/** Retrieve the current download queue. */
export async function getQueue(): Promise<DownloadJob[]> {
    return invoke<DownloadJob[]>("get_queue");
}

/** Remove a completed, failed, or cancelled download from the queue. */
export async function removeJob(jobId: string): Promise<void> {
    return invoke<void>("remove_job", { jobId });
}

// ========== Settings ==========

/** Retrieve the current application settings. */
export async function getSettings(): Promise<AppSettings> {
    return invoke<AppSettings>("get_settings");
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

/** Subscribe to download log events. Returns an unlisten function. */
export function onDownloadLog(
    callback: (payload: DownloadLogPayload) => void,
): Promise<UnlistenFn> {
    return listen<DownloadLogPayload>("download-log", (event) =>
        callback(event.payload),
    );
}

/** Subscribe to format-selected events. Returns an unlisten function. */
export function onFormatSelected(
    callback: (payload: FormatSelectedPayload) => void,
): Promise<UnlistenFn> {
    return listen<FormatSelectedPayload>("format-selected", (event) =>
        callback(event.payload),
    );
}

/** Validate a format expression and return matching format IDs. */
export async function validateFormatExpression(
    expression: string,
    formatIds: string[],
): Promise<string[]> {
    return invoke<string[]>("validate_format_expression", {
        expression,
        formatIds,
    });
}
