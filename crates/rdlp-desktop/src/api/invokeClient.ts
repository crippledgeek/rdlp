// Typed wrapper around Tauri invoke() with error normalization.
//
// All Rust command calls MUST go through invokeTyped<T>() instead of
// raw invoke() so errors have a consistent shape.

import { invoke } from "@tauri-apps/api/core";

/** Normalized error shape for all invoke failures. */
export interface InvokeError {
    code: string;
    message: string;
    details?: unknown;
}

/**
 * Extract a human-readable message from a Tauri command rejection.
 *
 * Tauri serializes Rust errors as plain JSON objects (no Error wrapper).
 * Our AppError uses `#[serde(tag = "kind", content = "data")]`, so the
 * JS rejection value looks like:
 *   `{ kind: "SearchFailed", data: { message: "...", retryable: true } }`
 */
function extractErrorMessage(err: unknown): string {
    if (err instanceof Error) return err.message;
    if (typeof err === "string") return err;
    if (typeof err === "object" && err !== null) {
        const obj = err as Record<string, unknown>;
        // AppError adjacently-tagged: { kind, data: { message } }
        if (typeof obj["data"] === "object" && obj["data"] !== null) {
            const data = obj["data"] as Record<string, unknown>;
            if (typeof data["message"] === "string") return data["message"];
        }
        // Plain object with top-level message
        if (typeof obj["message"] === "string") return obj["message"];
        // Last resort: readable JSON instead of [object Object]
        try { return JSON.stringify(err); } catch { /* fall through */ }
    }
    return String(err);
}

/**
 * Type-safe invoke wrapper with error normalization.
 *
 * @param command - The Rust `#[tauri::command]` name.
 * @param args - Arguments matching the command's parameters.
 * @returns The deserialized response from Rust.
 * @throws {InvokeError} on any failure.
 */
export async function invokeTyped<T>(
    command: string,
    args?: Record<string, unknown>,
): Promise<T> {
    try {
        return await invoke<T>(command, args);
    } catch (err: unknown) {
        const message = extractErrorMessage(err);
        throw { code: "INVOKE_ERROR", message, details: err } satisfies InvokeError;
    }
}
