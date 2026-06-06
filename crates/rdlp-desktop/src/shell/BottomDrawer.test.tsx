// Regression test for #336: a job in the `processing` (post-processing) state
// must surface in the BottomDrawer status line as the active job — it must NOT
// fall through to the idle "Ready" state. Before the isInFlight migration the
// status line gated on `status === "running"`, so a processing job looked idle.

import { describe, it, expect } from "vitest";
import { screen } from "@testing-library/react";
import { render, createTestQueryClient } from "@/test/test-utils";
import { BottomDrawer } from "./BottomDrawer";
import { downloadsQueryOptions } from "@/api/downloads";
import type { DownloadJob, JobStatus } from "@/types";

function makeJob(status: JobStatus, overrides: Partial<DownloadJob> = {}): DownloadJob {
    return {
        id: "job-1",
        url: "https://example.com/v",
        title: "Test Video",
        status,
        progress: 0.5,
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

function renderDrawer(jobs: DownloadJob[]) {
    const qc = createTestQueryClient();
    qc.setQueryData(downloadsQueryOptions().queryKey, jobs);
    return render(<BottomDrawer />, { queryClient: qc });
}

describe("BottomDrawer — processing job surfaces as active (#336)", () => {
    it("shows the processing job title in the status line, not the idle 'Ready' state", () => {
        renderDrawer([makeJob("processing", { title: "Processing Video" })]);
        const status = screen.getByTestId("drawer-status");
        // The active job title is shown...
        expect(status).toHaveTextContent("Processing Video");
        // ...and the idle "Ready" placeholder is NOT.
        expect(status).not.toHaveTextContent("Ready");
    });

    it("falls back to the 'Ready' state when no job is in flight", () => {
        renderDrawer([makeJob("completed", { title: "Done Video" })]);
        const status = screen.getByTestId("drawer-status");
        expect(status).toHaveTextContent("Ready");
        expect(status).not.toHaveTextContent("Done Video");
    });
});
