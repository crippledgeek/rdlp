// @vitest-environment node
// Tests for registerDownloadEvents — verifying cache mutations per event type.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { mockEmit, clearEventListeners } from "@/test/tauri-mock";
import { queryKeys } from "../query/queryKeys";
import { registerDownloadEvents } from "./registerDownloadEvents";
import type { DownloadJob } from "../types";

function makeJob(overrides: Partial<DownloadJob> = {}): DownloadJob {
    return {
        id: "job-1",
        url: "https://example.com/video",
        title: "Test Video",
        status: "running",
        progress: 0,
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
        stage: null,
        ...overrides,
    };
}

describe("registerDownloadEvents", () => {
    let qc: QueryClient;
    let cleanup: () => void;

    beforeEach(() => {
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { cb(0); return 1; });
        vi.stubGlobal("cancelAnimationFrame", () => {});
        qc = new QueryClient({
            defaultOptions: { queries: { retry: false, staleTime: Infinity } },
        });
        cleanup = registerDownloadEvents(qc);
    });

    afterEach(() => {
        cleanup();
        clearEventListeners();
        qc.clear();
        vi.unstubAllGlobals();
    });

    it("transitions a running job to cancelled on download-cancelled", async () => {
        const job = makeJob({
            id: "j1",
            status: "running",
            statusMessage: "Video — 47%",
            progress: 0.47,
            speed: "4.2 MB/s",
            eta: "0:12",
            currentUnit: { unitTitle: "Video", unitIndex: 1, unitTotal: 2 },
        });
        qc.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);

        await mockEmit("download-cancelled", { jobId: "j1" });

        const jobs = qc.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.status).toBe("cancelled");
        expect(jobs[0]!.statusMessage).toBeNull();
        expect(jobs[0]!.currentUnit).toBeNull();
        expect(jobs[0]!.speed).toBeNull();
        expect(jobs[0]!.eta).toBeNull();
        expect(jobs[0]!.progress).toBeNull();
    });

    it("appends a log message to logMessages on download-log event", async () => {
        const job = makeJob({ id: "job-42" });
        qc.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);

        await mockEmit("download-log", {
            jobId: "job-42",
            level: "info",
            message: "Fetching metadata",
        });

        const jobs = qc.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.logMessages).toEqual(["Fetching metadata"]);
    });

    it("accumulates multiple log messages in order", async () => {
        const job = makeJob({ id: "job-42" });
        qc.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);

        await mockEmit("download-log", {
            jobId: "job-42",
            level: "info",
            message: "First",
        });
        await mockEmit("download-log", {
            jobId: "job-42",
            level: "info",
            message: "Second",
        });
        await mockEmit("download-log", {
            jobId: "job-42",
            level: "warn",
            message: "Third",
        });

        const jobs = qc.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.logMessages).toEqual(["First", "Second", "Third"]);
    });

    it("also updates statusMessage on download-log event", async () => {
        const job = makeJob({ id: "job-42" });
        qc.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);

        await mockEmit("download-log", {
            jobId: "job-42",
            level: "info",
            message: "Merging streams",
        });

        const jobs = qc.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.statusMessage).toBe("Merging streams");
    });

    it("does not affect other jobs when a log event arrives", async () => {
        const job1 = makeJob({ id: "job-1" });
        const job2 = makeJob({ id: "job-2", title: "Other Video" });
        qc.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job1, job2]);

        await mockEmit("download-log", {
            jobId: "job-1",
            level: "info",
            message: "Only for job-1",
        });

        const jobs = qc.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.logMessages).toEqual(["Only for job-1"]);
        expect(jobs[1]!.logMessages).toBeUndefined();
    });

    it("appends to existing logMessages when already populated", async () => {
        const job = makeJob({ id: "job-42", logMessages: ["Previous message"] });
        qc.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);

        await mockEmit("download-log", {
            jobId: "job-42",
            level: "info",
            message: "New message",
        });

        const jobs = qc.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.logMessages).toEqual(["Previous message", "New message"]);
    });

    it("stores download-progress as a 0-1 fraction (not 0-100)", async () => {
        const job = makeJob({ id: "j1", status: "running" });
        qc.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);

        await mockEmit("download-progress", { jobId: "j1", progress: 0.47, speed: "5 MB/s", eta: "01:00" });

        const jobs = qc.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.progress).toBeCloseTo(0.47, 5);
    });
});
