import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { render } from "@/test/test-utils";
import { HistoryRow } from "./HistoryRow";
import { uiStore, setSelectedJob } from "@/stores/uiStore";
import { startDownload } from "@/api/downloads";
import { invokeTyped } from "@/api/invokeClient";
import type { DownloadJob } from "@/types";

vi.mock("@/stores/uiStore", async () => {
    const { Store } = await import("@tanstack/store");
    return {
        uiStore: new Store({ selectedJobId: null as string | null }),
        setSelectedJob: vi.fn(),
        setView: vi.fn(),
    };
});
vi.mock("@/api/downloads", () => ({ startDownload: vi.fn().mockResolvedValue("job-1") }));
vi.mock("@/api/invokeClient", () => ({ invokeTyped: vi.fn().mockResolvedValue(undefined) }));

function makeJob(overrides: Partial<DownloadJob> = {}): DownloadJob {
    return {
        id: "job-1", url: "https://example.com/video", title: "Done Video",
        status: "completed", progress: 1, speed: null, eta: null, error: null,
        retryable: false, started_at: null, completed_at: 1_700_000_000,
        output_path: "/tmp/v.mp4", options: null, playlist: null, currentUnit: null, stage: null,
        ...overrides,
    };
}

describe("HistoryRow — selection a11y", () => {
    beforeEach(() => { vi.clearAllMocks(); });

    it("exposes a Select button with accessible name + aria-pressed=false", () => {
        render(<HistoryRow job={makeJob({ id: "job-1", title: "Done Video" })} />);
        const sel = screen.getByRole("button", { name: "Select: Done Video" });
        expect(sel).toHaveAttribute("aria-pressed", "false");
    });

    it("exposes aria-pressed=true when selected", () => {
        uiStore.setState((prev) => ({ ...prev, selectedJobId: "job-1" }));
        try {
            render(<HistoryRow job={makeJob({ id: "job-1", title: "Done Video" })} />);
            expect(screen.getByRole("button", { name: "Select: Done Video" })).toHaveAttribute("aria-pressed", "true");
        } finally {
            uiStore.setState((prev) => ({ ...prev, selectedJobId: null }));
        }
    });

    it("Select button click selects the job", () => {
        render(<HistoryRow job={makeJob()} />);
        fireEvent.click(screen.getByRole("button", { name: /^Select:/ }));
        expect(vi.mocked(setSelectedJob)).toHaveBeenCalledWith("job-1");
    });

    it("Enter on the Select button selects the job", async () => {
        const user = userEvent.setup();
        render(<HistoryRow job={makeJob()} />);
        screen.getByRole("button", { name: /^Select:/ }).focus();
        await user.keyboard("{Enter}");
        expect(vi.mocked(setSelectedJob)).toHaveBeenCalledWith("job-1");
    });

    it("Space on the Select button selects the job", async () => {
        const user = userEvent.setup();
        render(<HistoryRow job={makeJob()} />);
        screen.getByRole("button", { name: /^Select:/ }).focus();
        await user.keyboard("{ }");
        expect(vi.mocked(setSelectedJob)).toHaveBeenCalledWith("job-1");
    });

    it("action buttons are NOT descendants of the Select button (axe nested-interactive guard)", () => {
        render(<HistoryRow job={makeJob({ output_path: "/tmp/v.mp4" })} />);
        const sel = screen.getByRole("button", { name: /^Select:/ });
        expect(screen.getByLabelText("Reveal in folder")).toBeInTheDocument();
        expect(sel.querySelector('[aria-label="Reveal in folder"]')).toBeNull();
    });

    it("Reveal calls reveal_in_folder and does NOT select", () => {
        render(<HistoryRow job={makeJob({ output_path: "/tmp/v.mp4" })} />);
        fireEvent.click(screen.getByLabelText("Reveal in folder"));
        expect(vi.mocked(invokeTyped)).toHaveBeenCalledWith("reveal_in_folder", { path: "/tmp/v.mp4" });
        expect(vi.mocked(setSelectedJob)).not.toHaveBeenCalled();
    });

    it("Re-download calls startDownload and does NOT select", () => {
        const job = makeJob({ options: {} as unknown as DownloadJob["options"], url: "u", title: "t" });
        render(<HistoryRow job={job} />);
        fireEvent.click(screen.getByLabelText("Re-download"));
        expect(vi.mocked(startDownload)).toHaveBeenCalledWith("u", job.options, "t");
        expect(vi.mocked(setSelectedJob)).not.toHaveBeenCalled();
    });

    it("Reveal hidden with no output_path; Re-download hidden with no options", () => {
        render(<HistoryRow job={makeJob({ output_path: null, options: null })} />);
        expect(screen.queryByLabelText("Reveal in folder")).not.toBeInTheDocument();
        expect(screen.queryByLabelText("Re-download")).not.toBeInTheDocument();
    });
});
