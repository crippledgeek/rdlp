import { describe, it, expect } from "vitest";
import {
    cancelledJobPatch,
    isTerminal,
    isDoneNotFailed,
    jobMatchesFilter,
    countByBucket,
    tallyByStatus,
} from "./jobStatus";
import type { JobStatus, DownloadJob } from "../types";

describe("cancelledJobPatch", () => {
    it("sets cancelled status and clears all transient progress fields", () => {
        expect(cancelledJobPatch()).toEqual({
            status: "cancelled",
            statusMessage: null,
            currentUnit: null,
            speed: null,
            eta: null,
            progress: null,
        });
    });
});

function makeJob(status: JobStatus, id = "j"): DownloadJob {
    return {
        id,
        url: "https://example.com/v",
        title: null,
        status,
        progress: null,
        speed: null,
        eta: null,
        error: null,
        retryable: false,
        started_at: null,
        completed_at: null,
        output_path: null,
        options: null,
        playlist: null,
        statusMessage: null,
    };
}

const ALL_STATUSES: JobStatus[] = ["pending", "running", "completed", "failed", "cancelled"];

describe("isTerminal", () => {
    it("is true for completed, cancelled, failed", () => {
        expect(isTerminal("completed")).toBe(true);
        expect(isTerminal("cancelled")).toBe(true);
        expect(isTerminal("failed")).toBe(true);
    });
    it("is false for running", () => {
        expect(isTerminal("running")).toBe(false);
    });
    it("is false for pending", () => {
        expect(isTerminal("pending")).toBe(false);
    });
});

describe("isDoneNotFailed", () => {
    it("is true for completed and cancelled", () => {
        expect(isDoneNotFailed("completed")).toBe(true);
        expect(isDoneNotFailed("cancelled")).toBe(true);
    });
    it("is false for failed (keeps the group expanded)", () => {
        expect(isDoneNotFailed("failed")).toBe(false);
    });
    it("is false for running and pending", () => {
        expect(isDoneNotFailed("running")).toBe(false);
        expect(isDoneNotFailed("pending")).toBe(false);
    });
});

describe("jobMatchesFilter", () => {
    it("matches each status to its own bucket", () => {
        expect(jobMatchesFilter("completed", "completed")).toBe(true);
        expect(jobMatchesFilter("running", "active")).toBe(true);
        expect(jobMatchesFilter("pending", "active")).toBe(true);
        expect(jobMatchesFilter("cancelled", "cancelled")).toBe(true);
    });
    it("'all' matches every status", () => {
        for (const s of ALL_STATUSES) {
            expect(jobMatchesFilter(s, "all")).toBe(true);
        }
    });
    it("does NOT match cancelled to completed (regression: #364)", () => {
        expect(jobMatchesFilter("cancelled", "completed")).toBe(false);
    });
    it("does NOT cross other buckets", () => {
        expect(jobMatchesFilter("completed", "cancelled")).toBe(false);
        expect(jobMatchesFilter("running", "completed")).toBe(false);
        expect(jobMatchesFilter("failed", "active")).toBe(false);
    });
});

describe("countByBucket", () => {
    it("counts a single completed job", () => {
        expect(countByBucket([makeJob("completed")])).toEqual({
            all: 1, active: 0, completed: 1, cancelled: 0, failed: 0,
        });
    });
    it("counts active = running + pending", () => {
        expect(countByBucket([makeJob("running"), makeJob("pending")])).toEqual({
            all: 2, active: 2, completed: 0, cancelled: 0, failed: 0,
        });
    });
    it("counts a mixed list into distinct buckets", () => {
        const jobs = [makeJob("running"), makeJob("completed"), makeJob("cancelled"), makeJob("failed")];
        expect(countByBucket(jobs)).toEqual({
            all: 4, active: 1, completed: 1, cancelled: 1, failed: 1,
        });
    });
    it("returns all zeros for an empty list", () => {
        expect(countByBucket([])).toEqual({
            all: 0, active: 0, completed: 0, cancelled: 0, failed: 0,
        });
    });
    it("partitions cleanly: all === active+completed+cancelled+failed", () => {
        const jobs = ALL_STATUSES.flatMap((s) => [makeJob(s), makeJob(s)]);
        const c = countByBucket(jobs);
        expect(c.all).toBe(c.active + c.completed + c.cancelled + c.failed);
    });
    it("does NOT count a cancelled job as completed (regression: #364)", () => {
        const c = countByBucket([makeJob("cancelled")]);
        expect(c.completed).toBe(0);
        expect(c.cancelled).toBe(1);
        expect(c.active).toBe(0);
        expect(c.failed).toBe(0);
    });
    it("counts only true completions among cancelled+completed", () => {
        const jobs = [makeJob("cancelled"), makeJob("cancelled"), makeJob("completed")];
        expect(countByBucket(jobs).completed).toBe(1);
    });
});

describe("tallyByStatus", () => {
    it("counts each status exactly", () => {
        const jobs = [makeJob("running"), makeJob("completed"), makeJob("cancelled")];
        expect(tallyByStatus(jobs)).toEqual({
            pending: 0, running: 1, completed: 1, failed: 0, cancelled: 1,
        });
    });
    it("returns all zeros for an empty list", () => {
        expect(tallyByStatus([])).toEqual({
            pending: 0, running: 0, completed: 0, failed: 0, cancelled: 0,
        });
    });
    it("a cancelled job increments only cancelled (regression guard)", () => {
        const t = tallyByStatus([makeJob("cancelled")]);
        expect(t.cancelled).toBe(1);
        expect(t.completed).toBe(0);
    });
});
