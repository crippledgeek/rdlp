// @vitest-environment node
// Unit tests for logStore.ts — the Log Viewer's bounded entry buffer.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
    logStore,
    appendLog,
    clearLogs,
    MAX_ENTRIES,
} from "./logStore";

// The production flush runs on an animation frame. Stubbing rAF to invoke its
// callback synchronously — the idiom registerDownloadEvents.test.ts uses —
// would publish once per append and make batching unobservable, which is the
// property under test. So frames are QUEUED here and drained on demand.
//
// `cancelAnimationFrame` must genuinely remove the queued callback. Stubbing it
// as a no-op (the easy mistake) makes cancellation unobservable no matter what
// the store does, so a test written against such a stub measures the stub
// rather than the code — and reports the same number whether the cancel is
// present or deleted.
let frames = new Map<number, FrameRequestCallback>();
let nextHandle = 1;

function drainFrames(): void {
    const queued = [...frames.values()];
    frames.clear();
    for (const cb of queued) cb(0);
}

/** Append `n` entries with distinguishable messages. */
function appendN(n: number, prefix = "m"): void {
    for (let i = 0; i < n; i++) appendLog("info", `${prefix}${i}`, null);
}

beforeEach(() => {
    frames = new Map();
    nextHandle = 1;
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        const handle = nextHandle++;
        frames.set(handle, cb);
        return handle;
    });
    vi.stubGlobal("cancelAnimationFrame", (handle: number) => {
        frames.delete(handle);
    });
    clearLogs();
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("batching", () => {
    it("publishes once for a burst of appends, not once per record", () => {
        let publishes = 0;
        // `subscribe` returns a Subscription, not a bare unsubscribe function.
        const subscription = logStore.subscribe(() => {
            publishes++;
        });

        appendN(50);
        expect(publishes).toBe(0); // nothing published until the frame runs

        drainFrames();
        expect(publishes).toBe(1); // 50 appends, one publish

        subscription.unsubscribe();
    });

    // Publishing the internal buffer by reference would pass every
    // single-flush assertion in this file: the content is right, and the bug
    // only shows on the SECOND flush, when later appends have mutated the
    // array a reader already holds. It also keeps the reference identical
    // across publishes, so `useStore` would never re-render the pane.
    it("publishes a snapshot that later appends cannot mutate", () => {
        appendN(3, "a");
        drainFrames();
        const firstSnapshot = logStore.state.entries;
        expect(firstSnapshot).toHaveLength(3);

        appendN(2, "b");
        drainFrames();

        expect(firstSnapshot).toHaveLength(3); // unchanged by the later appends
        expect(logStore.state.entries).toHaveLength(5);
        expect(logStore.state.entries).not.toBe(firstSnapshot); // new reference
    });

    it("makes every entry of the burst visible after the flush", () => {
        appendN(50);
        drainFrames();

        expect(logStore.state.entries).toHaveLength(50);
        expect(logStore.state.entries[0]?.message).toBe("m0");
        expect(logStore.state.entries[49]?.message).toBe("m49");
    });
});

describe("cap boundary", () => {
    // Both sides of the threshold. A far-from-boundary test (append 20000,
    // assert 5000) passes just as well against an off-by-one that drops an
    // entry at exactly the cap, so it would pin nothing.
    it("keeps every entry at exactly MAX_ENTRIES", () => {
        appendN(MAX_ENTRIES);
        drainFrames();

        const { entries } = logStore.state;
        expect(entries).toHaveLength(MAX_ENTRIES);
        expect(entries[0]?.message).toBe("m0"); // the oldest is still present
    });

    it("drops exactly the oldest entry at MAX_ENTRIES + 1", () => {
        appendN(MAX_ENTRIES + 1);
        drainFrames();

        const { entries } = logStore.state;
        expect(entries).toHaveLength(MAX_ENTRIES);
        expect(entries[0]?.message).toBe("m1"); // m0 evicted, m1 is now oldest
        expect(entries[MAX_ENTRIES - 1]?.message).toBe(`m${MAX_ENTRIES}`);
    });

    it("never exceeds the cap when the overflow is large", () => {
        appendN(MAX_ENTRIES * 2);
        drainFrames();

        expect(logStore.state.entries).toHaveLength(MAX_ENTRIES);
    });
});

describe("ordering and identity", () => {
    it("preserves FIFO order across an eviction", () => {
        appendN(MAX_ENTRIES + 10);
        drainFrames();

        const { entries } = logStore.state;
        // The surviving window is the NEWEST MAX_ENTRIES, oldest-first.
        expect(entries[0]?.message).toBe("m10");
        expect(entries[MAX_ENTRIES - 1]?.message).toBe(`m${MAX_ENTRIES + 9}`);
    });

    it("assigns strictly increasing ids", () => {
        appendN(100);
        drainFrames();

        const ids = logStore.state.entries.map((e) => e.id);
        for (let i = 1; i < ids.length; i++) {
            expect(ids[i]!).toBeGreaterThan(ids[i - 1]!);
        }
    });

    it("carries level, message and jobId through unchanged", () => {
        appendLog("warn", "disk nearly full", "job-7");
        drainFrames();

        expect(logStore.state.entries[0]).toMatchObject({
            level: "warn",
            message: "disk nearly full",
            jobId: "job-7",
        });
    });

    it("defaults jobId to null when omitted", () => {
        appendLog("info", "no owning job");
        drainFrames();

        expect(logStore.state.entries[0]?.jobId).toBeNull();
    });
});

describe("clearLogs", () => {
    it("empties the buffer", () => {
        appendN(10);
        drainFrames();
        expect(logStore.state.entries).toHaveLength(10);

        clearLogs();
        expect(logStore.state.entries).toHaveLength(0);
    });

    it("takes effect without waiting for a frame", () => {
        appendN(10);
        drainFrames();

        clearLogs();
        // No drainFrames() here: a user-initiated clear must be visible at
        // once, not on the next animation frame.
        expect(logStore.state.entries).toHaveLength(0);
    });

    it("discards appends that were still pending when it ran", () => {
        appendN(10); // queued, never flushed
        clearLogs();
        drainFrames(); // the stale frame must not resurrect them

        expect(logStore.state.entries).toHaveLength(0);
    });

    // The assertion above cannot fail if `clearLogs` forgets to cancel the
    // pending frame: that frame reads the emptied buffer and republishes the
    // same empty result, so the state is identical either way. What differs is
    // that subscribers are notified twice — one redundant re-render of the
    // pane per clear. Verified by mutation: deleting the cancel makes this
    // test, and only this test, fail.
    it("notifies subscribers once, not twice, when a flush was pending", () => {
        appendN(10);

        let publishes = 0;
        const subscription = logStore.subscribe(() => {
            publishes++;
        });

        clearLogs();
        drainFrames();

        expect(publishes).toBe(1);
        subscription.unsubscribe();
    });
});
