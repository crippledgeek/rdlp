import { describe, it, expect } from "vitest";
import {
    cancelledJobPatch,
    postProcessingJobPatch,
    isTerminal,
    isDoneNotFailed,
    isInFlight,
    jobMatchesFilter,
    countByBucket,
    tallyByStatus,
    jobSubLabel,
} from "./jobStatus";
import type { JobStatus, DownloadJob } from "../types";

describe("cancelledJobPatch", () => {
    it("sets cancelled status and clears all transient progress fields", () => {
        expect(cancelledJobPatch()).toEqual({
            status: "cancelled",
            currentUnit: null,
            speed: null,
            eta: null,
            progress: null,
        });
    });
});

describe("postProcessingJobPatch", () => {
    // Regression guard: during recode/remux the job MUST flip to the
    // "processing" bucket with its stage label — the postprocess-progress
    // reducer previously updated only progress/eta, leaving the badge stuck on
    // "Downloading" through the entire post-processing phase.
    it("flips status to processing and mirrors stage/progress/eta, clearing speed", () => {
        expect(postProcessingJobPatch("Recode", 0.42, "01:30")).toEqual({
            status: "processing",
            stage: "Recode",
            progress: 0.42,
            eta: "01:30",
            speed: null,
        });
    });

    it("passes through a null eta (pre-warm-up tick)", () => {
        expect(postProcessingJobPatch("Merging", 0, null)).toEqual({
            status: "processing",
            stage: "Merging",
            progress: 0,
            eta: null,
            speed: null,
        });
    });
});

function makeJob(status: JobStatus, id = "j", overrides: Partial<DownloadJob> = {}): DownloadJob {
    const base: DownloadJob = {
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
        currentUnit: null,
        stage: null,
    };
    return { ...base, status, id, ...overrides };
}

const ALL_STATUSES: JobStatus[] = ["pending", "running", "processing", "completed", "failed", "cancelled"];

describe("jobSubLabel", () => {
    it("shows stage, decimal percent, and ETA while processing", () => {
        expect(
            jobSubLabel(makeJob("processing", "a", { stage: "Recode", progress: 0.004, eta: "02:30" })),
        ).toBe("Recode — 0.4% · 02:30");
    });
    it("shows decimal percent without ETA before warm-up while processing", () => {
        expect(
            jobSubLabel(makeJob("processing", "a", { stage: "Recode", progress: 0.004, eta: null })),
        ).toBe("Recode — 0.4%");
    });
    it("falls back to 'Processing' when stage is null (no literal 'null')", () => {
        const label = jobSubLabel(makeJob("processing", "a", { stage: null, progress: 0.004, eta: null }));
        expect(label).toBe("Processing — 0.4%");
        expect(label).not.toContain("null");
    });
    it("formats a merge sub-unit with percent", () => {
        expect(jobSubLabel(makeJob("running", "a", { currentUnit: { index: 1, total: 2, title: "Video" }, progress: 0.67 }))).toBe("Video — 67%");
    });
    it("formats a playlist episode", () => {
        expect(jobSubLabel(makeJob("running", "a", { currentUnit: { index: 3, total: 12, title: "Ep title" } }))).toBe("Ep 3/12 — Ep title");
    });
    it("returns null with no stage and no unit", () => {
        expect(jobSubLabel(makeJob("running", "a"))).toBeNull();
    });
    it("treats total:2 with a non-Video/Audio title as playlist", () => {
        expect(jobSubLabel(makeJob("running", "a", { currentUnit: { index: 1, total: 2, title: "Bonus" } }))).toBe("Ep 1/2 — Bonus");
    });
    it("prefers stage over currentUnit while processing", () => {
        expect(
            jobSubLabel(
                makeJob("processing", "a", {
                    stage: "Merge",
                    progress: 0,
                    currentUnit: { index: 1, total: 2, title: "Video" },
                }),
            ),
        ).toBe("Merge — 0.0%");
    });
});

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
            all: 1, active: 0, processing: 0, completed: 1, cancelled: 0, failed: 0,
        });
    });
    it("counts active = running + pending", () => {
        expect(countByBucket([makeJob("running"), makeJob("pending")])).toEqual({
            all: 2, active: 2, processing: 0, completed: 0, cancelled: 0, failed: 0,
        });
    });
    it("counts a mixed list into distinct buckets", () => {
        const jobs = [makeJob("running"), makeJob("completed"), makeJob("cancelled"), makeJob("failed")];
        expect(countByBucket(jobs)).toEqual({
            all: 4, active: 1, processing: 0, completed: 1, cancelled: 1, failed: 1,
        });
    });
    it("returns all zeros for an empty list", () => {
        expect(countByBucket([])).toEqual({
            all: 0, active: 0, processing: 0, completed: 0, cancelled: 0, failed: 0,
        });
    });
    it("partitions cleanly: all === active+processing+completed+cancelled+failed", () => {
        const jobs = ALL_STATUSES.flatMap((s) => [makeJob(s), makeJob(s)]);
        const c = countByBucket(jobs);
        expect(c.all).toBe(c.active + c.processing + c.completed + c.cancelled + c.failed);
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
            pending: 0, running: 1, processing: 0, completed: 1, failed: 0, cancelled: 1,
        });
    });
    it("returns all zeros for an empty list", () => {
        expect(tallyByStatus([])).toEqual({
            pending: 0, running: 0, processing: 0, completed: 0, failed: 0, cancelled: 0,
        });
    });
    it("a cancelled job increments only cancelled (regression guard)", () => {
        const t = tallyByStatus([makeJob("cancelled")]);
        expect(t.cancelled).toBe(1);
        expect(t.completed).toBe(0);
    });
});

describe("processing bucket + isInFlight", () => {
    it("counts a processing job in its own bucket", () => {
        expect(countByBucket([makeJob("processing")])).toEqual({
            all: 1, active: 0, processing: 1, completed: 0, cancelled: 0, failed: 0,
        });
    });
    it("counts a mixed list across all buckets", () => {
        const jobs = [makeJob("running"), makeJob("processing"), makeJob("completed"), makeJob("cancelled"), makeJob("failed")];
        expect(countByBucket(jobs)).toEqual({ all: 5, active: 1, processing: 1, completed: 1, cancelled: 1, failed: 1 });
    });
    it("partitions cleanly incl. processing", () => {
        const jobs = ALL_STATUSES.map((s) => makeJob(s));
        const c = countByBucket(jobs);
        expect(c.all).toBe(c.active + c.processing + c.completed + c.cancelled + c.failed);
    });
    it("processing matches only its own filter", () => {
        expect(jobMatchesFilter("processing", "processing")).toBe(true);
        expect(jobMatchesFilter("processing", "all")).toBe(true);
        expect(jobMatchesFilter("processing", "active")).toBe(false);
        expect(jobMatchesFilter("processing", "completed")).toBe(false);
    });
    it("tallyByStatus counts processing", () => {
        expect(tallyByStatus([makeJob("processing")]).processing).toBe(1);
        expect(tallyByStatus([]).processing).toBe(0);
    });
    it("processing is not terminal", () => {
        expect(isTerminal("processing")).toBe(false);
        expect(isDoneNotFailed("processing")).toBe(false);
    });
    it("isInFlight is true for running and processing only", () => {
        expect(isInFlight("running")).toBe(true);
        expect(isInFlight("processing")).toBe(true);
        expect(isInFlight("pending")).toBe(false);
        expect(isInFlight("completed")).toBe(false);
        expect(isInFlight("failed")).toBe(false);
        expect(isInFlight("cancelled")).toBe(false);
    });
    it("countByBucket does not throw for processing", () => {
        expect(() => countByBucket([makeJob("processing")])).not.toThrow();
    });
});
