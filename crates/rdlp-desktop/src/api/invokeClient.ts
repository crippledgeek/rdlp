// Typed wrapper around Tauri invoke() with error normalization.
//
// All Rust command calls MUST go through invokeTyped() instead of raw
// invoke() so errors have a consistent shape and the command name, its
// arguments and its result type are checked against the one command map
// (`ipc.ts`). `scripts/check-ipc-command-drift.sh` also pins `invoke` imports
// to this file.

import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import type { IpcArgTuple, IpcCommand, IpcCommands, IpcResult } from "./ipc";

/** Normalized error shape for all invoke failures. */
export interface InvokeError {
    code: string;
    message: string;
    details?: unknown;
}

/**
 * Extract a human-readable message from a Tauri command rejection.
 *
 * Exported because the UI needs it too: a rejection reaching a React error
 * branch is an `InvokeError` plain object, not an `Error`, so `(err as
 * Error).message` is a cast the type system cannot back. Views call this
 * instead.
 *
 * Tauri serializes Rust errors as plain JSON objects (no Error wrapper).
 * Our AppError uses `#[serde(tag = "kind", content = "data")]`, so the
 * JS rejection value looks like:
 *   `{ kind: "SearchFailed", data: { message: "...", retryable: true } }`
 */
export function extractErrorMessage(err: unknown): string {
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
 * `K` is inferred from the command-name literal; the argument object and the
 * result type follow from `IpcCommands[K]`, so neither is a claim the caller
 * makes (the old `invokeTyped<T>` "DECLARED `T` but never validated it").
 *
 * @param command - A Rust `#[tauri::command]` name from `IpcCommands`.
 * @param rest - The command's arguments — omitted for a `void`-args command.
 * @returns The deserialized response from Rust.
 * @throws {InvokeError} on any failure.
 */
export async function invokeTyped<K extends IpcCommand>(
    command: K,
    ...rest: IpcArgTuple<K>
): IpcResult<K> {
    try {
        // `rest[0]` is the typed args object or `undefined`; Tauri's `InvokeArgs`
        // is the wider wire type it is sent as.
        return await invoke<IpcCommands[K]["result"]>(command, rest[0] as InvokeArgs | undefined);
    } catch (err: unknown) {
        const message = extractErrorMessage(err);
        throw { code: "INVOKE_ERROR", message, details: err } satisfies InvokeError;
    }
}
