// Tests for views/queue/JobCard — display-layer progress rendering.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import { render } from "@/test/test-utils";
import { JobCard } from "./JobCard";
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
vi.mock("@/api/invokeClient", () => ({
    invokeTyped: vi.fn(),
}));

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
});
