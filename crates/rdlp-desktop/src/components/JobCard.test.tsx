import { render, screen } from "@/test/test-utils";
import userEvent from "@testing-library/user-event";
import { JobCard } from "./JobCard";
import type { DownloadJob } from "../types";

function makeJob(overrides: Partial<DownloadJob> = {}): DownloadJob {
    return {
        id: "job-1",
        url: "https://example.com/video",
        title: "Test Video",
        status: "pending",
        progress: null,
        speed: null,
        eta: null,
        error: null,
        retryable: false,
        started_at: null,
        completed_at: null,
        output_path: null,
        options: null,
        statusMessage: null,
        ...overrides,
    };
}

describe("JobCard", () => {
    it("renders job title when provided", () => {
        render(
            <JobCard
                job={makeJob({ title: "My Video" })}
                onCancel={vi.fn()}
                onRemove={vi.fn()}
                onRetry={vi.fn()}
            />,
        );
        expect(screen.getByText("My Video")).toBeInTheDocument();
    });

    it("falls back to URL when title is null", () => {
        render(
            <JobCard
                job={makeJob({ title: null, url: "https://example.com/fallback" })}
                onCancel={vi.fn()}
                onRemove={vi.fn()}
                onRetry={vi.fn()}
            />,
        );
        expect(screen.getByText("https://example.com/fallback")).toBeInTheDocument();
    });

    it("renders status badge", () => {
        render(
            <JobCard
                job={makeJob({ status: "running" })}
                onCancel={vi.fn()}
                onRemove={vi.fn()}
                onRetry={vi.fn()}
            />,
        );
        expect(screen.getByText("running")).toBeInTheDocument();
    });

    it("shows Cancel button while running", () => {
        render(
            <JobCard
                job={makeJob({ status: "running", progress: 0.5 })}
                onCancel={vi.fn()}
                onRemove={vi.fn()}
                onRetry={vi.fn()}
            />,
        );
        expect(screen.getByRole("button", { name: /cancel/i })).toBeInTheDocument();
    });

    it("calls onCancel with job id when Cancel is clicked", async () => {
        const onCancel = vi.fn();
        render(
            <JobCard
                job={makeJob({ status: "running", progress: 0.3 })}
                onCancel={onCancel}
                onRemove={vi.fn()}
                onRetry={vi.fn()}
            />,
        );
        await userEvent.click(screen.getByRole("button", { name: /cancel/i }));
        expect(onCancel).toHaveBeenCalledWith("job-1");
    });

    it("shows progress percentage while running", () => {
        render(
            <JobCard
                job={makeJob({ status: "running", progress: 0.75 })}
                onCancel={vi.fn()}
                onRemove={vi.fn()}
                onRetry={vi.fn()}
            />,
        );
        expect(screen.getByText("75.0%")).toBeInTheDocument();
    });

    it("shows Remove button for terminal statuses", async () => {
        const onRemove = vi.fn();
        render(
            <JobCard
                job={makeJob({ status: "completed" })}
                onCancel={vi.fn()}
                onRemove={onRemove}
                onRetry={vi.fn()}
            />,
        );
        const removeBtn = screen.getByRole("button", { name: /remove/i });
        await userEvent.click(removeBtn);
        expect(onRemove).toHaveBeenCalledWith("job-1");
    });

    it("shows Retry button for retryable failed jobs", async () => {
        const onRetry = vi.fn();
        render(
            <JobCard
                job={makeJob({ status: "failed", retryable: true, error: "Network error" })}
                onCancel={vi.fn()}
                onRemove={vi.fn()}
                onRetry={onRetry}
            />,
        );
        await userEvent.click(screen.getByRole("button", { name: /retry/i }));
        // onRetry now receives the full job object so the caller can preserve options
        expect(onRetry).toHaveBeenCalledWith(
            expect.objectContaining({ url: "https://example.com/video" }),
        );
    });

    it("displays error message for failed jobs", () => {
        render(
            <JobCard
                job={makeJob({ status: "failed", error: "Download timed out" })}
                onCancel={vi.fn()}
                onRemove={vi.fn()}
                onRetry={vi.fn()}
            />,
        );
        expect(screen.getByText("Download timed out")).toBeInTheDocument();
    });
});
