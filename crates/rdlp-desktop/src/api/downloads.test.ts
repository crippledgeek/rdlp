import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("./invokeClient", () => ({ invokeTyped: (...a: unknown[]) => invokeMock(...a) }));

import { cancelDownload } from "./downloads";
import { queryClient } from "../query/queryClient";
import { queryKeys } from "../query/queryKeys";
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
        currentUnit: null,
        stage: null,
        ...overrides,
    };
}

describe("cancelDownload", () => {
    beforeEach(() => {
        invokeMock.mockReset();
        invokeMock.mockResolvedValue(undefined);
        queryClient.clear();
    });

    it("optimistically flips the job to cancelled without refetching", async () => {
        const job = makeJob({
            id: "j1",
            status: "running",
            progress: 0.47,
            speed: "4.2 MB/s",
            eta: "0:12",
            currentUnit: { index: 1, total: 2, title: "Video" },
        });
        queryClient.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);
        const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

        await cancelDownload("j1");

        const jobs = queryClient.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.status).toBe("cancelled");
        expect(jobs[0]!.currentUnit).toBeNull();
        expect(jobs[0]!.speed).toBeNull();
        expect(jobs[0]!.eta).toBeNull();
        expect(jobs[0]!.progress).toBeNull();
        expect(invalidateSpy).not.toHaveBeenCalled();
    });
});

describe("cancelDownload — optimistic guard (#368)", () => {
    it("optimistically flips to cancelled on success", async () => {
        const key = queryKeys.downloads.list();
        queryClient.setQueryData(key, [makeJob({ id: "a", status: "running" })]);
        invokeMock.mockResolvedValueOnce(undefined);
        await cancelDownload("a");
        expect(queryClient.getQueryData<DownloadJob[]>(key)![0]!.status).toBe("cancelled");
    });
    it("flips optimistically BEFORE the IPC resolves", async () => {
        const key = queryKeys.downloads.list();
        queryClient.setQueryData(key, [makeJob({ id: "a", status: "running" })]);
        // Hold the IPC open so we can observe the in-flight optimistic state.
        let release!: () => void;
        invokeMock.mockReturnValueOnce(new Promise<void>((res) => (release = res)));
        const p = cancelDownload("a");
        // The documented pattern guards via cancelQueries() then writes the optimistic
        // flip — all before the cancel_download IPC resolves. Drain pending microtasks
        // (cancelQueries settles) without releasing the held IPC, then assert the flip.
        await new Promise((r) => setTimeout(r, 0));
        expect(queryClient.getQueryData<DownloadJob[]>(key)![0]!.status).toBe("cancelled");
        release();
        await p;
    });
    it("rolls back when the IPC rejects", async () => {
        const key = queryKeys.downloads.list();
        queryClient.setQueryData(key, [makeJob({ id: "a", status: "running" })]);
        invokeMock.mockRejectedValueOnce(new Error("boom"));
        await expect(cancelDownload("a")).rejects.toThrow("boom");
        expect(queryClient.getQueryData<DownloadJob[]>(key)![0]!.status).toBe("running"); // rolled back
    });
});
