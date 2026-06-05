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
        statusMessage: null,
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
            statusMessage: "Video — 47%",
            currentUnit: { unitIndex: 1, unitTotal: 2, unitTitle: "Video" },
        });
        queryClient.setQueryData<DownloadJob[]>(queryKeys.downloads.list(), [job]);
        const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

        await cancelDownload("j1");

        const jobs = queryClient.getQueryData<DownloadJob[]>(queryKeys.downloads.list())!;
        expect(jobs[0]!.status).toBe("cancelled");
        expect(jobs[0]!.statusMessage).toBeNull();
        expect(jobs[0]!.currentUnit).toBeNull();
        expect(jobs[0]!.speed).toBeNull();
        expect(jobs[0]!.eta).toBeNull();
        expect(jobs[0]!.progress).toBeNull();
        expect(invalidateSpy).not.toHaveBeenCalled();
    });
});
