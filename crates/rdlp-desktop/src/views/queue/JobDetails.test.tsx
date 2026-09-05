import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { render, createTestQueryClient } from "@/test/test-utils";
import { JobDetails } from "./JobDetails";
import { downloadsQueryOptions } from "@/api/downloads";
import { uiStore, setSelectedJob } from "@/stores/uiStore";
import { invokeTyped } from "@/api/invokeClient";
import { toast } from "@/components/ui/sonner";
import type { DownloadJob } from "@/types";

// Only `invokeTyped` is faked; `extractErrorMessage` stays real so these
// assertions exercise the unwrap the app actually ships.
vi.mock("@/api/invokeClient", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/api/invokeClient")>()),
    invokeTyped: vi.fn(),
}));
vi.mock("@/components/ui/sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

function completedJob(): DownloadJob {
    return {
        id: "job-1",
        url: "https://example.com/v",
        title: "A Video",
        status: "completed",
        progress: 1,
        speed: null,
        eta: null,
        error: null,
        retryable: false,
        started_at: null,
        completed_at: null,
        output_path: "/tmp/v.mp4",
        options: null,
        playlist: null,
        currentUnit: null,
        stage: null,
    };
}

function renderDetails() {
    const qc = createTestQueryClient();
    qc.setQueryData(downloadsQueryOptions().queryKey, [completedJob()]);
    setSelectedJob("job-1");
    return render(<JobDetails />, { queryClient: qc });
}

describe("JobDetails — Reveal in Folder", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        uiStore.setState((s) => ({ ...s, selectedJobId: null }));
    });

    it("invokes reveal_in_folder with the job's output path", async () => {
        vi.mocked(invokeTyped).mockResolvedValueOnce(undefined);
        renderDetails();
        await userEvent.click(screen.getByRole("button", { name: /Reveal in Folder/i }));
        expect(vi.mocked(invokeTyped)).toHaveBeenCalledWith("reveal_in_folder", {
            path: "/tmp/v.mp4",
        });
    });

    // The negative path is the point of the change: before #693 this handler
    // reported only to `console.error`, which is invisible in a packaged
    // build, so a failed reveal looked exactly like a successful one.
    it("surfaces a failure as a toast", async () => {
        vi.mocked(invokeTyped).mockRejectedValueOnce({
            message: "File not found: /tmp/v.mp4",
        });
        renderDetails();
        await userEvent.click(screen.getByRole("button", { name: /Reveal in Folder/i }));
        await waitFor(() =>
            expect(vi.mocked(toast.error)).toHaveBeenCalledWith("File not found: /tmp/v.mp4"),
        );
    });

    // `extractErrorMessage` returns "" for an empty message and "{}" for an
    // empty object — the latter is truthy, so the `||` guard exists for the
    // former. An AppError with a blank message is the shape that reaches it.
    it("falls back to a generic message when the error's message is empty", async () => {
        vi.mocked(invokeTyped).mockRejectedValueOnce({ message: "" });
        renderDetails();
        await userEvent.click(screen.getByRole("button", { name: /Reveal in Folder/i }));
        await waitFor(() =>
            expect(vi.mocked(toast.error)).toHaveBeenCalledWith("Failed to reveal the file"),
        );
    });
});
