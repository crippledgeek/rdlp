// TanStack Store for the Log Viewer's bounded entry buffer.
//
// Two producers write here: `registerLogEvents` forwards every record from the
// Rust `log` facade, and `registerDownloadEvents` forwards per-job
// `download-log` messages. `LogViewer` is the only reader.
//
// The write path and the published snapshot are deliberately separate. Records
// arrive in bursts — an extraction or a segmented download emits many within a
// single frame — so appends go straight into a fixed-size circular buffer and
// one snapshot is published per animation frame. Publishing per record instead
// would rebuild the whole array up to `MAX_ENTRIES` times per frame to render
// one set of rows. `registerDownloadEvents` batches its progress updates on a
// frame for the same reason.

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

// A real circular buffer, not an array that gets trimmed.
//
// The distinction is load-bearing. Trimming makes the bound conditional on
// something running — and the obvious places to put that something are both
// wrong. On append it costs an O(n) copy per record, which is what this module
// exists to avoid. In the flush it is worse than it looks: `requestAnimationFrame`
// callbacks do not run while the document is not being rendered (HTML spec), so
// a minimized or occluded window keeps accepting records from Rust with no
// flush to trim them, and the buffer grows without limit for as long as the
// window stays hidden — a long download in the background being the exact case
// the pane exists to serve.
//
// Fixed slots remove the question. Memory is exactly `MAX_ENTRIES` entries at
// all times, whether a frame ever arrives or not, and there is no trim branch
// that a later edit can delete without any test noticing — which is how both
// earlier attempts at this failed.
const ring: Array<LogEntry | undefined> = new Array(MAX_ENTRIES);
let head = 0; // slot the next record goes into
let count = 0; // records currently held, never above MAX_ENTRIES
let nextId = 0;
let frameHandle: number | null = null;

/** Materialize the held records, oldest first. */
function snapshot(): LogEntry[] {
    const out: LogEntry[] = new Array(count);
    const start = (head - count + MAX_ENTRIES) % MAX_ENTRIES;
    for (let i = 0; i < count; i++) {
        // Non-null: exactly `count` slots ahead of `start` have been written.
        out[i] = ring[(start + i) % MAX_ENTRIES]!;
    }
    return out;
}

/**
 * Schedule a publish on the next animation frame, if one is not already due.
 *
 * Every environment this runs in supplies `requestAnimationFrame` — the Tauri
 * webview, happy-dom in the component tests, and a stub in the node-environment
 * suites. Where it is somehow absent, publish synchronously: that loses
 * batching and nothing else. The previous microtask fallback was worse than no
 * fallback, being both unreachable in practice and subtly wrong — a queued
 * microtask cannot be cancelled, so `clearLogs` could not suppress it, and
 * since `setState` always propagates a fresh object it delivered exactly the
 * redundant subscriber notification the cancel exists to prevent.
 */
function scheduleFlush(): void {
    if (frameHandle !== null) return;

    if (typeof requestAnimationFrame !== "function") {
        flush();
        return;
    }

    frameHandle = requestAnimationFrame(() => {
        frameHandle = null;
        flush();
    });
}

function cancelPendingFlush(): void {
    if (frameHandle === null) return;
    cancelAnimationFrame(frameHandle);
    frameHandle = null;
}

function flush(): void {
    // Always a fresh array: the next append would otherwise mutate published
    // state in place, and `useStore` compares by reference, so the pane would
    // not re-render.
    logStore.setState(() => ({ entries: snapshot() }));
}

/**
 * Append a line to the buffer.
 *
 * Visible to readers on the next flush, not synchronously — see the batching
 * note at the top of this file. O(1): one slot write, no copy.
 */
export function appendLog(
    level: LogEntry["level"],
    message: string,
    jobId: string | null = null,
): void {
    ring[head] = { id: nextId++, timestamp: Date.now(), level, jobId, message };
    head = (head + 1) % MAX_ENTRIES;
    if (count < MAX_ENTRIES) count++;

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
    // Release the entries rather than only resetting the indices, so clearing
    // a full pane actually frees them instead of keeping 5000 objects alive in
    // slots nothing will read.
    //
    // No test covers this line, and none can through this module's surface:
    // dropping it leaves every published result identical and only changes
    // when the entries become collectable. Verified by mutation — it is the
    // one change here that survives the suite.
    ring.fill(undefined);
    head = 0;
    count = 0;

    cancelPendingFlush();
    logStore.setState(() => ({ entries: [] }));
}
