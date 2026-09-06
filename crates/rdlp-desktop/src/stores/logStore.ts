// TanStack Store for the Log Viewer's bounded entry buffer.
//
// Two producers write here: `registerLogEvents` forwards every record from the
// Rust `log` facade, and `registerDownloadEvents` forwards per-job
// `download-log` messages. `LogViewer` is the only reader.
//
// The write path and the published snapshot are deliberately separate. Records
// arrive in bursts — an extraction or a segmented download emits many within a
// single frame — so appends land in a mutable array and one copy is published
// per animation frame. Publishing per record instead would copy the whole
// buffer up to `MAX_ENTRIES` times per frame to render one set of rows.
// `registerDownloadEvents` batches its progress updates on a frame for the
// same reason.

import { Store } from "@tanstack/store";

/** A single rendered log line. */
export interface LogEntry {
    id: number;
    timestamp: number;
    level: "info" | "warn" | "debug" | "error";
    jobId: string | null;
    message: string;
}

/**
 * Entries retained for the pane.
 *
 * The log file on disk is the complete record; this is a tail deep enough to
 * cover a session's worth of scrollback without holding the process's entire
 * log history in memory.
 */
export const MAX_ENTRIES = 5000;

export interface LogState {
    entries: LogEntry[];
}

export const logStore = new Store<LogState>({ entries: [] });

// Owned by this module and never handed out: readers only ever see the copy
// published to the store, so a mutation here cannot be observed mid-flight.
let buffer: LogEntry[] = [];
let nextId = 0;
let frameHandle: number | null = null;

/**
 * Schedule a publish on the next animation frame, if one is not already due.
 *
 * `requestAnimationFrame` is absent outside a browser (a `node`-environment
 * test that has not stubbed it), so fall back to a microtask rather than
 * throwing — a store that cannot be imported under test is worse than one that
 * flushes on a slightly different schedule.
 */
function scheduleFlush(): void {
    if (frameHandle !== null) return;

    if (typeof requestAnimationFrame === "function") {
        frameHandle = requestAnimationFrame(() => {
            frameHandle = null;
            flush();
        });
        return;
    }

    frameHandle = -1;
    queueMicrotask(() => {
        frameHandle = null;
        flush();
    });
}

function cancelPendingFlush(): void {
    if (frameHandle === null) return;
    if (frameHandle !== -1 && typeof cancelAnimationFrame === "function") {
        cancelAnimationFrame(frameHandle);
    }
    // The microtask path cannot be cancelled; `flush` is idempotent against an
    // emptied buffer, so a stale one publishes the (already correct) state.
    frameHandle = null;
}

function flush(): void {
    // Enforce the cap here rather than on append, so `appendLog` stays O(1).
    // Between flushes the buffer may hold one frame's worth of extra records;
    // this is what bounds it, and it bounds the published snapshot with it —
    // deleting this branch would publish an unbounded array, which is what
    // makes the cap tests able to catch its loss.
    if (buffer.length > MAX_ENTRIES) {
        buffer = buffer.slice(buffer.length - MAX_ENTRIES);
    }

    // A copy, never `buffer` itself: the next append would otherwise mutate
    // published state in place, and `useStore` would not see a new reference.
    logStore.setState(() => ({ entries: buffer.slice() }));
}

/**
 * Append a line to the buffer.
 *
 * Visible to readers on the next flush, not synchronously — see the batching
 * note at the top of this file.
 */
export function appendLog(
    level: LogEntry["level"],
    message: string,
    jobId: string | null = null,
): void {
    buffer.push({ id: nextId++, timestamp: Date.now(), level, jobId, message });
    scheduleFlush();
}

/**
 * Empty the buffer.
 *
 * Publishes synchronously and drops any pending flush: this is driven by the
 * pane's clear button, and a user who clears the log should not watch a queued
 * frame put the lines back.
 */
export function clearLogs(): void {
    buffer = [];
    cancelPendingFlush();
    logStore.setState(() => ({ entries: [] }));
}
