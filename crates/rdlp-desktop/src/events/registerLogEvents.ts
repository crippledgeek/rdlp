// Bridge from the Rust `log` facade into the Log Viewer's ring buffer.
//
// Called once at module scope from main.tsx, next to `attachConsole()` — not
// from a component effect. The listener is tied to the window's lifetime
// rather than any component's, and registering before the shell mounts keeps
// startup records that an effect would miss.
//
// tauri-plugin-log's `Webview` target emits every record the facade accepts as
// a "log://log" event. `attachConsole()` in main.tsx is one subscriber to that
// event, forwarding to the devtools console; this module is the second, and it
// is what puts those records in front of a user who is not holding devtools
// open. Reusing the plugin's own event is why this needs no custom
// `TargetKind::Dispatch`, no second event name, and no `log:` capability entry
// — an event listener is not a plugin command.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { onLogRecord } from "../lib/tauri";
import { appendLog } from "../components/LogViewer";
import type { LogEntry } from "../components/LogViewer";

/**
 * Translate tauri-plugin-log's numeric level into the Log Viewer's severity
 * union.
 *
 * The plugin serializes `LogLevel` as `repr(u16)` — Trace=1, Debug=2, Info=3,
 * Warn=4, Error=5 (tauri-plugin-log 2.9.1, src/lib.rs:79-101).
 *
 * Trace folds into `debug` rather than widening the union: the pane's toolbar
 * renders one button per member, and a fifth would be a control for a level
 * this app never emits — `LOG_LEVEL` in src-tauri/src/lib.rs is `Info`.
 *
 * An unrecognised integer means the plugin changed its repr. The fallback is
 * `info` rather than a passthrough because LogViewer filters with
 * `activeFilters.has(entry.level)`: a value outside the union matches no
 * filter and the row would vanish instead of merely looking wrong.
 */
export function severityFromPluginLevel(level: number): LogEntry["level"] {
    switch (level) {
        case 1:
        case 2:
            return "debug";
        case 4:
            return "warn";
        case 5:
            return "error";
        default:
            return "info";
    }
}

/**
 * Register the `log://log` listener.
 *
 * Returns a cleanup function that unregisters it. The production call site
 * discards it, exactly as main.tsx discards `attachConsole()`'s detach handle;
 * it exists because a subscribe function that cannot be undone is untestable,
 * and the tests use it to prove teardown works.
 *
 * The listener is registered asynchronously, so the teardown chains off the
 * same promise rather than assuming it has resolved.
 */
export function registerLogEvents(): () => void {
    let mounted = true;

    const unlisten: Promise<UnlistenFn> = onLogRecord((payload) => {
        if (!mounted) return;
        // `jobId` is null: a facade record belongs to no particular download.
        // The per-job stream is `download-log`, handled in
        // registerDownloadEvents.
        appendLog(severityFromPluginLevel(payload.level), payload.message, null);
    });

    return () => {
        mounted = false;
        void unlisten.then((fn) => fn());
    };
}
