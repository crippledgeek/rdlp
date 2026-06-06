import { describe, it, expect, beforeEach } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { render, createTestQueryClient } from "@/test/test-utils";
import { QueueNav } from "./QueueNav";
import { downloadsQueryOptions } from "@/api/downloads";
import { queueFilterStore, setQueueFilter } from "@/stores/queueFilterStore";
import type { DownloadJob, JobStatus } from "@/types";

function makeJob(status: JobStatus, id: string): DownloadJob {
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

function renderNav(jobs: DownloadJob[]) {
    const qc = createTestQueryClient();
    qc.setQueryData(downloadsQueryOptions().queryKey, jobs);
    return render(<QueueNav />, { queryClient: qc });
}

/** Read the numeric value of a STATS row by its term (dt) label. */
function statValue(label: string): string {
    const term = screen.getByText(label, { selector: "dt" });
    const row = term.closest("div");
    if (!row) throw new Error(`no row for ${label}`);
    return within(row).getByRole("definition").textContent ?? "";
}

describe("QueueNav — STATS bucketing (#364)", () => {
    beforeEach(() => {
        setQueueFilter("all");
    });

    it("counts a cancelled job under Cancelled, not Completed", () => {
        renderNav([makeJob("cancelled", "a")]);
        expect(statValue("Completed")).toBe("0");
        expect(statValue("Cancelled")).toBe("1");
    });

    it("shows distinct counts for a mixed queue", () => {
        renderNav([
            makeJob("completed", "a"),
            makeJob("cancelled", "b"),
            makeJob("failed", "c"),
            makeJob("running", "d"),
        ]);
        expect(statValue("Total")).toBe("4");
        expect(statValue("Active")).toBe("1");
        expect(statValue("Completed")).toBe("1");
        expect(statValue("Cancelled")).toBe("1");
        expect(statValue("Failed")).toBe("1");
    });

    it("renders a Cancelled filter tab with an accessible, counted label", () => {
        renderNav([makeJob("cancelled", "a")]);
        const tab = screen.getByRole("button", { name: /cancelled/i });
        expect(tab).toHaveAttribute("aria-label", expect.stringMatching(/cancelled.*1 item/i));
    });

    it("does not fold a cancelled job into the Completed chip", () => {
        renderNav([makeJob("cancelled", "a")]);
        // Completed chip: count 0 → aria-label has no item count
        const completedTab = screen.getByRole("button", { name: "Filter by Completed downloads" });
        expect(completedTab).toBeInTheDocument();
        // Cancelled chip: count 1 → aria-label carries the count
        const cancelledTab = screen.getByRole("button", { name: /Filter by Cancelled downloads, 1 item/i });
        expect(cancelledTab).toBeInTheDocument();
    });

    it("activates the cancelled filter on click", async () => {
        renderNav([makeJob("cancelled", "a")]);
        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: /cancelled/i }));
        expect(queueFilterStore.state.filter).toBe("cancelled");
    });

    it("shows 'Clear Finished' for a cancelled-only queue", () => {
        renderNav([makeJob("cancelled", "a")]);
        expect(screen.getByRole("button", { name: /clear finished/i })).toBeInTheDocument();
    });

    it("hides the clear button when only active jobs exist", () => {
        renderNav([makeJob("running", "a")]);
        expect(screen.queryByRole("button", { name: /clear finished/i })).not.toBeInTheDocument();
    });
});
