// @vitest-environment node
// Tests for registerLogEvents — the bridge from tauri-plugin-log's `log://log`
// event into the shared log store.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mockEmit, clearEventListeners, listenerCount } from "@/test/tauri-mock";

const appendLog = vi.fn();

vi.mock("@/stores/logStore", () => ({
    appendLog: (...args: unknown[]) => appendLog(...args),
}));

const { registerLogEvents, severityFromPluginLevel } = await import("./registerLogEvents");

describe("severityFromPluginLevel", () => {
    // tauri-plugin-log serializes `LogLevel` as a repr(u16): Trace=1, Debug=2,
    // Info=3, Warn=4, Error=5 (plugin src/lib.rs:79-101).
    it.each([
        [1, "debug"],
        [2, "debug"],
        [3, "info"],
        [4, "warn"],
        [5, "error"],
    ])("maps plugin level %i to %s", (level, expected) => {
        expect(severityFromPluginLevel(level)).toBe(expected);
    });

    // The fold point. Trace collapses into debug rather than widening the
    // pane's four-member union, so these two must stay on opposite sides: an
    // off-by-one in the mapping moves Info into debug and hides every ordinary
    // record behind a filter button that is off by default in nobody's setup
    // but reads as the wrong severity.
    it("keeps the trace/debug fold distinct from info", () => {
        expect(severityFromPluginLevel(1)).toBe("debug");
        expect(severityFromPluginLevel(3)).toBe("info");
    });

    // An unrecognised integer means the plugin changed its repr under us. The
    // row must stay visible and filterable: `undefined` would fail every
    // `activeFilters.has(...)` check in LogViewer and drop the record silently.
    it.each([0, 6, -1, 99, Number.NaN])(
        "falls back to info for out-of-range level %s",
        (level) => {
            expect(severityFromPluginLevel(level)).toBe("info");
        },
    );
});

describe("registerLogEvents", () => {
    let cleanup: () => void;

    beforeEach(() => {
        appendLog.mockClear();
        cleanup = registerLogEvents();
    });

    afterEach(() => {
        cleanup();
        clearEventListeners();
    });

    it("forwards a log://log record into the log store with no job id", async () => {
        await mockEmit("log://log", {
            message: "[rdlp_extractor] resolved 3 formats",
            level: 3,
        });

        expect(appendLog).toHaveBeenCalledTimes(1);
        expect(appendLog).toHaveBeenCalledWith(
            "info",
            "[rdlp_extractor] resolved 3 formats",
            null,
        );
    });

    it("carries the severity through for a warning", async () => {
        await mockEmit("log://log", { message: "slow response", level: 4 });

        expect(appendLog).toHaveBeenCalledWith("warn", "slow response", null);
    });

    it("stops forwarding after cleanup", async () => {
        cleanup();
        // The listener is registered asynchronously and torn down the same
        // way; let the unlisten promise settle before emitting.
        await Promise.resolve();
        await Promise.resolve();

        await mockEmit("log://log", { message: "after teardown", level: 3 });

        expect(appendLog).not.toHaveBeenCalled();
    });

    // Deliberately separate from the assertion above, which the `mounted`
    // guard alone satisfies: delete the unlisten call and the handler still
    // stops appending, while staying subscribed to the event forever. Only the
    // listener count can see that leak, and the cleanup exists for no other
    // reason than to prevent it.
    it("detaches the listener on cleanup, not merely silences it", async () => {
        expect(listenerCount("log://log")).toBe(1);

        cleanup();
        await Promise.resolve();
        await Promise.resolve();

        expect(listenerCount("log://log")).toBe(0);
    });
});
