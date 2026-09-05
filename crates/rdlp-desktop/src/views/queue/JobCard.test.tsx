// Tests for views/queue/JobCard — display-layer progress rendering.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { render } from "@/test/test-utils";
import { JobCard } from "./JobCard";
import { uiStore, setSelectedJob } from "@/stores/uiStore";
import { cancelDownload, removeJob, startDownload } from "@/api/downloads";
import { invokeTyped } from "@/api/invokeClient";
import { toast } from "@/components/ui/sonner";
import type { DownloadJob } from "@/types";

// Stub Tauri stores and IPC — JobCard reads uiStore + calls Tauri commands.
vi.mock("@/stores/uiStore", async () => {
    const { Store } = await import("@tanstack/store");
    return {
        uiStore: new Store({ selectedJobId: null as string | null }),
        setSelectedJob: vi.fn(),
    };
});
vi.mock("@/api/downloads", () => ({
    cancelDownload: vi.fn(),
    removeJob: vi.fn(),
    startDownload: vi.fn(),
}));
vi.mock("@/api/invokeClient", async (importOriginal) => ({
    // Only `invokeTyped` is faked. `extractErrorMessage` stays real so the
    // toast assertions below exercise the actual unwrap the app ships.
    ...(await importOriginal<typeof import("@/api/invokeClient")>()),
    invokeTyped: vi.fn(),
}));
vi.mock("@/components/ui/sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

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

describe("JobCard — progress display", () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    it("renders a 0-1 progress fraction as whole percent", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.47 })} />);
        expect(screen.getByText("47%")).toBeInTheDocument();
    });

    it("sizes the progress bar width to the 0-1 fraction scaled to percent", () => {
        const { container } = render(
            <JobCard job={makeJob({ status: "running", progress: 0.47 })} />,
        );
        // The fill bar is the only element with an inline width style. Guards the
        // higher-risk width path against a double-conversion (e.g. 4700%).
        const bar = container.querySelector('[style*="width"]') as HTMLElement | null;
        expect(bar).not.toBeNull();
        expect(bar!.style.width).toBe("47%");
    });

    it("renders 0 progress as 0%", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0 })} />);
        expect(screen.getByText("0%")).toBeInTheDocument();
    });

    it("renders 1.0 progress as 100%", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 1.0 })} />);
        expect(screen.getByText("100%")).toBeInTheDocument();
    });

    it("renders the title when present", () => {
        render(<JobCard job={makeJob({ title: "My Video" })} />);
        expect(screen.getByText("My Video")).toBeInTheDocument();
    });

    it("renders 'Unknown' for a null title (not the URL)", () => {
        render(<JobCard job={makeJob({ title: null, url: "https://x.example/y" })} />);
        expect(screen.getByText("Unknown")).toBeInTheDocument();
        expect(screen.queryByText("https://x.example/y")).not.toBeInTheDocument();
    });

    it("renders 'Unknown' for an empty-string title", () => {
        render(<JobCard job={makeJob({ title: "" })} />);
        expect(screen.getByText("Unknown")).toBeInTheDocument();
    });

    it("renders 'Unknown' for a whitespace-only title", () => {
        render(<JobCard job={makeJob({ title: "   " })} />);
        expect(screen.getByText("Unknown")).toBeInTheDocument();
    });

    it("compact mode renders 'Ep N — Unknown' for a null title", () => {
        const job = makeJob({
            title: null,
            playlist: { playlistId: "p", playlistTitle: "P", playlistIndex: 2, playlistCount: 5 },
        });
        render(<JobCard job={job} compact />);
        expect(screen.getByText("Ep 2 — Unknown")).toBeInTheDocument();
    });

    it("shows a 'retry may not help' hint on a non-retryable failure", () => {
        render(<JobCard job={makeJob({ status: "failed", error: "403 Forbidden", retryable: false })} />);
        expect(screen.getByText("403 Forbidden")).toBeInTheDocument();
        expect(screen.getByText(/retry may not help/)).toBeInTheDocument();
    });

    it("omits the hint when the failure is retryable", () => {
        render(<JobCard job={makeJob({ status: "failed", error: "503 Service Unavailable", retryable: true })} />);
        expect(screen.getByText("503 Service Unavailable")).toBeInTheDocument();
        expect(screen.queryByText(/retry may not help/)).not.toBeInTheDocument();
    });

    it("does not show the hint on a non-failed job", () => {
        render(<JobCard job={makeJob({ status: "running", retryable: false })} />);
        expect(screen.queryByText(/retry may not help/)).not.toBeInTheDocument();
    });

    it("does not show the hint on a completed job", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/v.mp4" })} />);
        expect(screen.queryByText(/retry may not help/)).not.toBeInTheDocument();
    });

    it("renders no hint when failed with no error message", () => {
        render(<JobCard job={makeJob({ status: "failed", error: null, retryable: false })} />);
        expect(screen.queryByText(/retry may not help/)).not.toBeInTheDocument();
    });

    it("shows the status badge label for the job status", () => {
        render(<JobCard job={makeJob({ status: "failed", error: "x" })} />);
        expect(screen.getByText("Failed")).toBeInTheDocument();
    });

    it("compact playlist mode renders an 'Ep N — title' label", () => {
        const job = makeJob({
            title: "Episode One",
            playlist: { playlistId: "p", playlistTitle: "P", playlistIndex: 3, playlistCount: 10 },
        });
        render(<JobCard job={job} compact />);
        expect(screen.getByText("Ep 3 — Episode One")).toBeInTheDocument();
    });

    it("Cancel (running) calls cancelDownload with the id", () => {
        render(<JobCard job={makeJob({ status: "running" })} />);
        fireEvent.click(screen.getByLabelText("Cancel download"));
        expect(vi.mocked(cancelDownload)).toHaveBeenCalledWith("job-1");
    });

    it("Retry shows on failed+options even when NOT retryable, and calls startDownload", () => {
        const job = makeJob({ status: "failed", retryable: false, url: "u", title: "t",
            options: {} as unknown as DownloadJob["options"] });
        render(<JobCard job={job} />);
        fireEvent.click(screen.getByLabelText("Retry download"));
        expect(vi.mocked(startDownload)).toHaveBeenCalledWith("u", job.options, "t");
    });

    it("Retry is hidden on a failed job with no options", () => {
        render(<JobCard job={makeJob({ status: "failed", options: null })} />);
        expect(screen.queryByLabelText("Retry download")).not.toBeInTheDocument();
    });

    it("Reveal (completed+output_path) calls reveal_in_folder", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/v.mp4" })} />);
        fireEvent.click(screen.getByLabelText("Reveal in folder"));
        expect(vi.mocked(invokeTyped)).toHaveBeenCalledWith("reveal_in_folder", { path: "/tmp/v.mp4" });
    });

    // The reveal command can fail for reasons the user can act on (the file
    // moved or was deleted since the download). Before #693 this handler had
    // no catch at all, so a failure was indistinguishable from success.
    it("Reveal surfaces a failure as a toast", async () => {
        vi.mocked(invokeTyped).mockRejectedValueOnce({
            message: "File not found: /tmp/v.mp4",
        });
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/v.mp4" })} />);
        fireEvent.click(screen.getByLabelText("Reveal in folder"));
        await waitFor(() =>
            expect(vi.mocked(toast.error)).toHaveBeenCalledWith("File not found: /tmp/v.mp4"),
        );
    });

    // A rejection whose message is blank must still say something: an empty
    // string is what the `|| fallback` guard is for (an empty *object* yields
    // "{}" from extractErrorMessage, which is truthy and bypasses it).
    it("Reveal falls back to a generic message when the error's message is empty", async () => {
        vi.mocked(invokeTyped).mockRejectedValueOnce({ message: "" });
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/v.mp4" })} />);
        fireEvent.click(screen.getByLabelText("Reveal in folder"));
        await waitFor(() =>
            expect(vi.mocked(toast.error)).toHaveBeenCalledWith("Failed to reveal the file"),
        );
    });

    it("Reveal is hidden when completed with no output_path", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: null })} />);
        expect(screen.queryByLabelText("Reveal in folder")).not.toBeInTheDocument();
    });

    it("Select button click selects the job", () => {
        render(<JobCard job={makeJob({ title: "Pick Me", status: "running" })} />);
        fireEvent.click(screen.getByRole("button", { name: /^Select:/ }));
        expect(vi.mocked(setSelectedJob)).toHaveBeenCalledWith("job-1");
    });

    it("exposes a Select button with accessible name and aria-pressed=false when unselected", () => {
        render(<JobCard job={makeJob({ id: "job-1", title: "Pick Me" })} />);
        const sel = screen.getByRole("button", { name: "Select: Pick Me" });
        expect(sel).toHaveAttribute("aria-pressed", "false");
    });

    it("exposes aria-pressed=true when the job is selected", () => {
        uiStore.setState((prev) => ({ ...prev, selectedJobId: "job-1" }));
        try {
            render(<JobCard job={makeJob({ id: "job-1", title: "Pick Me" })} />);
            const sel = screen.getByRole("button", { name: "Select: Pick Me" });
            expect(sel).toHaveAttribute("aria-pressed", "true");
        } finally {
            uiStore.setState((prev) => ({ ...prev, selectedJobId: null }));
        }
    });

    it("Enter on the Select button selects the job", async () => {
        const user = userEvent.setup();
        render(<JobCard job={makeJob({ title: "Pick Me", status: "running" })} />);
        screen.getByRole("button", { name: /^Select:/ }).focus();
        await user.keyboard("{Enter}");
        expect(vi.mocked(setSelectedJob)).toHaveBeenCalledWith("job-1");
    });

    it("Space on the Select button selects the job", async () => {
        const user = userEvent.setup();
        render(<JobCard job={makeJob({ title: "Pick Me", status: "running" })} />);
        screen.getByRole("button", { name: /^Select:/ }).focus();
        await user.keyboard("{ }");
        expect(vi.mocked(setSelectedJob)).toHaveBeenCalledWith("job-1");
    });

    it("action buttons are NOT descendants of the Select button (axe nested-interactive guard)", () => {
        render(<JobCard job={makeJob({ status: "running" })} />);
        const sel = screen.getByRole("button", { name: /^Select:/ });
        expect(screen.getByLabelText("Cancel download")).toBeInTheDocument();
        expect(sel.querySelector('[aria-label="Cancel download"]')).toBeNull();
    });

    it("a button click does not also select the job", () => {
        render(<JobCard job={makeJob({ title: "Pick Me", status: "running" })} />);
        fireEvent.click(screen.getByLabelText("Cancel download"));
        expect(vi.mocked(setSelectedJob)).not.toHaveBeenCalled();
    });

    it.each(["completed", "failed", "cancelled", "pending"] as const)(
        "Cancel is hidden when status is %s",
        (status) => {
            render(<JobCard job={makeJob({ status, output_path: "/t.mp4", error: "e" })} />);
            expect(screen.queryByLabelText("Cancel download")).not.toBeInTheDocument();
        },
    );

    it.each(["running", "completed", "cancelled", "pending"] as const)(
        "Retry is hidden when status is %s",
        (status) => {
            render(<JobCard job={makeJob({ status, options: {} as unknown as DownloadJob["options"], output_path: "/t.mp4" })} />);
            expect(screen.queryByLabelText("Retry download")).not.toBeInTheDocument();
        },
    );

    it.each(["running", "failed"] as const)(
        "Reveal is hidden when status is %s",
        (status) => {
            render(<JobCard job={makeJob({ status, output_path: "/t.mp4", error: "e" })} />);
            expect(screen.queryByLabelText("Reveal in folder")).not.toBeInTheDocument();
        },
    );

    it.each(["completed", "failed", "cancelled"] as const)(
        "Remove is shown when status is %s",
        (status) => {
            render(<JobCard job={makeJob({ status, output_path: "/t.mp4", error: "e" })} />);
            expect(screen.getByLabelText("Remove job")).toBeInTheDocument();
        },
    );

    it("Remove (terminal) calls removeJob", () => {
        render(<JobCard job={makeJob({ status: "cancelled" })} />);
        fireEvent.click(screen.getByLabelText("Remove job"));
        expect(vi.mocked(removeJob)).toHaveBeenCalledWith("job-1");
    });

    it.each(["running", "pending"] as const)(
        "Remove is hidden when status is %s",
        (status) => {
            render(<JobCard job={makeJob({ status })} />);
            expect(screen.queryByLabelText("Remove job")).not.toBeInTheDocument();
        },
    );

    it("shows the output-path text for completed (non-compact)", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/v.mp4" })} />);
        expect(screen.getByText("/tmp/v.mp4")).toBeInTheDocument();
    });

    it("hides the output-path text in compact mode", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/v.mp4" })} compact />);
        expect(screen.queryByText("/tmp/v.mp4")).not.toBeInTheDocument();
    });

    it("shows the progress percent while running", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.42 })} />);
        expect(screen.getByText("42%")).toBeInTheDocument();
    });

    it.each(["completed", "failed", "cancelled", "pending"] as const)(
        "hides the progress percent when status is %s",
        (status) => {
            render(<JobCard job={makeJob({ status, progress: 0.42, output_path: "/t.mp4", error: "e" })} />);
            expect(screen.queryByText("42%")).not.toBeInTheDocument();
        },
    );

    it("shows speed and ETA when running and present", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.5, speed: "4.2 MB/s", eta: "00:30" })} />);
        expect(screen.getByText("4.2 MB/s")).toBeInTheDocument();
        expect(screen.getByText("ETA 00:30")).toBeInTheDocument();
    });

    it("omits speed and ETA when null", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.5, speed: null, eta: null })} />);
        expect(screen.queryByText(/MB\/s/)).not.toBeInTheDocument();
        expect(screen.queryByText(/^ETA /)).not.toBeInTheDocument();
    });

    it("shows the derived sub-label for a merge unit when running", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.5, currentUnit: { index: 1, total: 2, title: "Video" } })} />);
        expect(screen.getByText("Video — 50%")).toBeInTheDocument();
    });
});

describe("JobCard — segmented estimate + frag counter (#381)", () => {
    it("renders a ~-prefixed estimated total size with an accessible label", () => {
        render(
            <JobCard
                job={makeJob({
                    status: "running",
                    progress: 0.5,
                    isEstimated: true,
                    totalBytes: 1_288_490_188, // ~1.2 GB
                })}
            />,
        );
        // Visible tilde + formatted size; the tilde is aria-hidden and paired
        // with an sr-only "approximately" so screen readers don't say "tilde".
        expect(screen.getByText("1.2 GB")).toBeInTheDocument();
        expect(screen.getByText("~")).toHaveAttribute("aria-hidden", "true");
        expect(screen.getByText(/approximately/i)).toBeInTheDocument();
    });

    it("does NOT render the ~ estimate marker for an exact download", () => {
        render(
            <JobCard
                job={makeJob({
                    status: "running",
                    progress: 0.5,
                    isEstimated: false,
                    totalBytes: 1_288_490_188,
                })}
            />,
        );
        expect(screen.queryByText("~")).not.toBeInTheDocument();
        expect(screen.queryByText(/approximately/i)).not.toBeInTheDocument();
    });

    it("renders the frag N/M counter when segment counts are present", () => {
        render(
            <JobCard
                job={makeJob({
                    status: "running",
                    progress: 0.3,
                    segmentsDownloaded: 12,
                    totalSegments: 40,
                })}
            />,
        );
        expect(screen.getByText("frag 12/40")).toBeInTheDocument();
    });

    it("omits the frag counter when segment counts are absent (progressive HTTP)", () => {
        render(
            <JobCard
                job={makeJob({
                    status: "running",
                    progress: 0.3,
                    segmentsDownloaded: null,
                    totalSegments: null,
                })}
            />,
        );
        expect(screen.queryByText(/^frag /)).not.toBeInTheDocument();
    });
});

describe("JobCard — processing (#336)", () => {
    it("shows the progress bar, decimal percent, and Cancel for a processing job", () => {
        render(<JobCard job={makeJob({ status: "processing", progress: 0.3, stage: "Recode" })} />);
        expect(screen.getByLabelText("Cancel download")).toBeInTheDocument();
        expect(screen.getByText("30.0%")).toBeInTheDocument();
    });
    it("shows the stage with decimal percent as the sub-label while processing", () => {
        render(<JobCard job={makeJob({ status: "processing", progress: 0.004, stage: "Recode" })} />);
        expect(screen.getByText("Recode — 0.4%")).toBeInTheDocument();
    });
    it("shows the ETA in the sub-label while processing once available", () => {
        render(
            <JobCard
                job={makeJob({ status: "processing", progress: 0.004, stage: "Recode", eta: "02:30" })}
            />,
        );
        expect(screen.getByText("Recode — 0.4% · 02:30")).toBeInTheDocument();
    });
    it("shows the ETA only in the sub-label (no separate ETA chip) while processing", () => {
        render(
            <JobCard
                job={makeJob({ status: "processing", progress: 0.004, stage: "Recode", eta: "02:30" })}
            />,
        );
        // The dedicated "ETA <value>" footer chip is suppressed for processing jobs.
        expect(screen.queryByText(/^ETA /)).not.toBeInTheDocument();
        // The ETA value still appears, but only embedded in the sub-label.
        expect(screen.getByText("Recode — 0.4% · 02:30")).toBeInTheDocument();
        expect(screen.getAllByText(/02:30/)).toHaveLength(1);
    });
    it("a running job still shows the dedicated 'ETA <value>' chip (guard against over-suppression)", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.5, eta: "00:30" })} />);
        expect(screen.getByText("ETA 00:30")).toBeInTheDocument();
    });
    it("a completed job shows no Cancel button", () => {
        render(<JobCard job={makeJob({ status: "completed", progress: 1 })} />);
        expect(screen.queryByLabelText("Cancel download")).not.toBeInTheDocument();
    });
    it("a running job still shows progress + Cancel", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.5 })} />);
        expect(screen.getByLabelText("Cancel download")).toBeInTheDocument();
        expect(screen.getByText("50%")).toBeInTheDocument();
    });
});
