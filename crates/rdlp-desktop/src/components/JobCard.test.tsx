import { render, screen, waitFor, fireEvent } from "@/test/test-utils";
import { setInvokeHandler, clearInvokeHandlers } from "@/test/tauri-mock";
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
        playlist: null,
        statusMessage: null,
        ...overrides,
    };
}

const noop = vi.fn();

describe("JobCard", () => {
    it("renders job title when provided", () => {
        render(<JobCard job={makeJob({ title: "My Video" })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByText("My Video")).toBeInTheDocument();
    });

    it("falls back to URL when title is null", () => {
        render(<JobCard job={makeJob({ title: null, url: "https://example.com/fallback" })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByText("https://example.com/fallback")).toBeInTheDocument();
    });

    it("renders status badge", () => {
        render(<JobCard job={makeJob({ status: "running" })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByText("running")).toBeInTheDocument();
    });

    it("shows Cancel button while running", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.5 })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByRole("button", { name: /cancel/i })).toBeInTheDocument();
    });

    it("calls onCancel with job id when Cancel is clicked", () => {
        const onCancel = vi.fn();
        render(<JobCard job={makeJob({ status: "running", progress: 0.3 })} onCancel={onCancel} onRemove={noop} onRetry={noop} />);
        fireEvent.click(screen.getByRole("button", { name: /cancel/i }));
        expect(onCancel).toHaveBeenCalledWith("job-1");
    });

    it("shows progress percentage while running", () => {
        render(<JobCard job={makeJob({ status: "running", progress: 0.75 })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByText("75.0%")).toBeInTheDocument();
    });

    it("shows Remove button for terminal statuses", () => {
        const onRemove = vi.fn();
        render(<JobCard job={makeJob({ status: "completed" })} onCancel={noop} onRemove={onRemove} onRetry={noop} />);
        fireEvent.click(screen.getByRole("button", { name: /remove/i }));
        expect(onRemove).toHaveBeenCalledWith("job-1");
    });

    it("shows Retry button for retryable failed jobs", () => {
        const onRetry = vi.fn();
        render(<JobCard job={makeJob({ status: "failed", retryable: true, error: "Network error" })} onCancel={noop} onRemove={noop} onRetry={onRetry} />);
        fireEvent.click(screen.getByRole("button", { name: /retry/i }));
        expect(onRetry).toHaveBeenCalledWith(
            expect.objectContaining({ url: "https://example.com/video" }),
        );
    });

    it("displays error message for failed jobs", () => {
        render(<JobCard job={makeJob({ status: "failed", error: "Download timed out" })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByText("Download timed out")).toBeInTheDocument();
    });

    it("shows Reveal in Folder button for completed jobs with output_path", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/video.mp4" })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByRole("button", { name: /reveal in folder/i })).toBeInTheDocument();
    });

    it("does not show Reveal in Folder button when output_path is null", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: null })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.queryByRole("button", { name: /reveal in folder/i })).not.toBeInTheDocument();
    });

    it("does not show Reveal in Folder button when output_path is empty string", () => {
        render(<JobCard job={makeJob({ status: "completed", output_path: "" })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.queryByRole("button", { name: /reveal in folder/i })).not.toBeInTheDocument();
    });

    it("shows error when reveal_in_folder fails", async () => {
        setInvokeHandler("reveal_in_folder", () => {
            throw { kind: "Internal", data: { message: "File not found" } };
        });
        render(<JobCard job={makeJob({ status: "completed", output_path: "/tmp/missing.mp4" })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        fireEvent.click(screen.getByRole("button", { name: /reveal in folder/i }));
        await waitFor(() => {
            expect(screen.getByText(/could not reveal file/i)).toBeInTheDocument();
        });
        clearInvokeHandlers();
    });

    it("does not render log panel when logMessages is undefined", () => {
        render(<JobCard job={makeJob()} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.queryByText(/^logs/i)).not.toBeInTheDocument();
    });

    it("does not render log panel when logMessages is empty", () => {
        render(<JobCard job={makeJob({ logMessages: [] })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.queryByText(/^logs/i)).not.toBeInTheDocument();
    });

    it("renders log panel summary with message count", () => {
        render(<JobCard job={makeJob({ logMessages: ["Starting download", "Fetching metadata"] })} onCancel={noop} onRemove={noop} onRetry={noop} />);
        expect(screen.getByText("Logs (2)")).toBeInTheDocument();
    });

    it("renders all log messages in the panel", () => {
        render(
            <JobCard
                job={makeJob({ logMessages: ["First message", "Second message", "Third message"] })}
                onCancel={noop} onRemove={noop} onRetry={noop}
            />,
        );
        fireEvent.click(screen.getByText("Logs (3)"));
        expect(screen.getByText("First message")).toBeInTheDocument();
        expect(screen.getByText("Second message")).toBeInTheDocument();
        expect(screen.getByText("Third message")).toBeInTheDocument();
    });
});
