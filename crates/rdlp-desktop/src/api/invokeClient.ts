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
        const message = err instanceof Error ? err.message : String(err);
        throw { code: "INVOKE_ERROR", message, details: err } satisfies InvokeError;
    }
}
