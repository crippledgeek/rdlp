// Typed wrappers around Tauri's listen() event subscriptions.
//
// Event listeners mirror the Tauri events emitted from src-tauri/src/events.rs.
// Commands are NOT wrapped here: every `#[tauri::command]` call goes through
// `api/invokeClient.ts`'s `invokeTyped`, keyed by the one command map in
// `api/ipc.ts`.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
    DownloadCancelledPayload,
    DownloadCompletePayload,
    DownloadErrorPayload,
    DownloadLogPayload,
    DownloadProgressPayload,
    LogRecordPayload,
    PostProcessProgressPayload,
    UnitStartedPayload,
} from "../types";

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

/**
 * Subscribe to Rust `log` facade records forwarded by tauri-plugin-log's
 * `Webview` target. Returns an unlisten function.
 *
 * Distinct from `onDownloadLog`, which carries per-job download messages. This
 * one carries everything any crate logs, and is what makes the Log Viewer a
 * log sink rather than a download-message stream.
 */
export function onLogRecord(
    callback: (payload: LogRecordPayload) => void,
): Promise<UnlistenFn> {
    return listen<LogRecordPayload>("log://log", (event) => callback(event.payload));
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
