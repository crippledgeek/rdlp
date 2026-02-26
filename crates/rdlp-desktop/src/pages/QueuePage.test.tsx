import { render, screen, waitFor, createTestQueryClient } from "@/test/test-utils";
import userEvent from "@testing-library/user-event";
import { setInvokeHandler, clearInvokeHandlers } from "@/test/tauri-mock";
import { QueuePage } from "./QueuePage";
import { queryKeys } from "../query/queryKeys";
import type { AppSettings, DownloadJob } from "../types";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const defaultSettings: AppSettings = {
    output_dir: "/home/user/Videos",
    default_remux: null,
    default_extract_audio: null,
    default_subtitle_format: null,
    default_subtitle_langs: [],
    embed_thumbnail: true,
    write_thumbnail: false,
    embed_metadata: true,
    verbose: false,
    default_search_provider: null,
    output_template: null,
    cookies_from_browser: null,
    cookies_file: null,
    proxy: null,
    rate_limit: null,
    normalize_audio: false,
    audio_gain_target: null,
    loudnorm: false,
    loudnorm_preset: null,
    loudnorm_target_i: null,
    loudnorm_target_tp: null,
    loudnorm_target_lra: null,
    loudnorm_dynamic: false,
    loudnorm_precompress: false,
    normalize_boost: false,
    normalize_boost_db: null,
    embed_subtitles: false,
};

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

/** Pre-seed the query cache so QueuePage renders synchronously. */
function seededClient(jobs: DownloadJob[] = []) {
    const qc = createTestQueryClient();
    qc.setQueryData(queryKeys.settings(), defaultSettings);
    qc.setQueryData(queryKeys.downloads.list(), jobs);
    return qc;
}

// ---------------------------------------------------------------------------
// Setup / teardown
// ---------------------------------------------------------------------------

beforeEach(() => {
    setInvokeHandler("get_settings", () => defaultSettings);
    setInvokeHandler("get_queue", () => []);
    setInvokeHandler("cancel_download", () => undefined);
    setInvokeHandler("remove_job", () => undefined);
    setInvokeHandler("clear_completed_jobs", () => undefined);
    setInvokeHandler("start_download", () => "new-job-id");
});

afterEach(() => {
    clearInvokeHandlers();
});

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

describe("QueuePage — empty state", () => {
    it("shows empty state when there are no jobs", () => {
        render(<QueuePage />, { queryClient: seededClient() });
        expect(screen.getByText(/no downloads yet/i)).toBeInTheDocument();
    });

    it("does not render Active or History headings when queue is empty", () => {
        render(<QueuePage />, { queryClient: seededClient() });
        expect(screen.getByText(/no downloads yet/i)).toBeInTheDocument();
        expect(screen.queryByText(/^active/i)).not.toBeInTheDocument();
        expect(screen.queryByText(/^history/i)).not.toBeInTheDocument();
    });
});

// ---------------------------------------------------------------------------
// Active / history split
// ---------------------------------------------------------------------------

describe("QueuePage — active/history split", () => {
    it("renders Active section for pending and running jobs", () => {
        const jobs = [
            makeJob({ id: "j1", status: "pending" }),
            makeJob({ id: "j2", status: "running", progress: 0.5 }),
        ];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^active \(2\)/i)).toBeInTheDocument();
        expect(screen.queryByText(/^history/i)).not.toBeInTheDocument();
    });

    it("renders History section for completed, failed, and cancelled jobs", () => {
        const jobs = [
            makeJob({ id: "j1", status: "completed" }),
            makeJob({ id: "j2", status: "failed", error: "timeout" }),
            makeJob({ id: "j3", status: "cancelled" }),
        ];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^history \(3\)/i)).toBeInTheDocument();
        expect(screen.queryByText(/^active/i)).not.toBeInTheDocument();
    });

    it("renders both Active and History sections when both exist", () => {
        const jobs = [
            makeJob({ id: "j1", status: "running", progress: 0.2 }),
            makeJob({ id: "j2", status: "completed" }),
        ];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^active \(1\)/i)).toBeInTheDocument();
        expect(screen.getByText(/^history \(1\)/i)).toBeInTheDocument();
    });

    it("renders a Cancel button only for active jobs", () => {
        const jobs = [
            makeJob({ id: "j1", status: "running", progress: 0.3 }),
            makeJob({ id: "j2", status: "completed" }),
        ];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^active \(1\)/i)).toBeInTheDocument();
        expect(screen.getAllByRole("button", { name: /cancel/i })).toHaveLength(1);
    });

    it("renders a Remove button for each terminal job", () => {
        const jobs = [
            makeJob({ id: "j1", status: "completed" }),
            makeJob({ id: "j2", status: "cancelled" }),
        ];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^history \(2\)/i)).toBeInTheDocument();
        expect(screen.getAllByRole("button", { name: /remove/i })).toHaveLength(2);
    });
});

// ---------------------------------------------------------------------------
// Cancel action
// ---------------------------------------------------------------------------

describe("QueuePage — cancel", () => {
    it("calls cancel_download with the correct job id", async () => {
        const cancelHandler = vi.fn((_args?: Record<string, unknown>) => undefined);
        setInvokeHandler("cancel_download", cancelHandler);
        const jobs = [makeJob({ id: "job-abc", status: "running", progress: 0.5 })];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        await userEvent.click(screen.getByRole("button", { name: /cancel/i }));
        await waitFor(() => expect(cancelHandler).toHaveBeenCalled());
        const args = cancelHandler.mock.calls[0][0] as { jobId: string };
        expect(args.jobId).toBe("job-abc");
    });
});

// ---------------------------------------------------------------------------
// Remove action
// ---------------------------------------------------------------------------

describe("QueuePage — remove", () => {
    it("calls remove_job with the correct job id", async () => {
        const removeHandler = vi.fn((_args?: Record<string, unknown>) => undefined);
        setInvokeHandler("remove_job", removeHandler);
        const jobs = [makeJob({ id: "job-xyz", status: "completed" })];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        await userEvent.click(screen.getByRole("button", { name: /remove/i }));
        await waitFor(() => expect(removeHandler).toHaveBeenCalled());
        const args = removeHandler.mock.calls[0][0] as { jobId: string };
        expect(args.jobId).toBe("job-xyz");
    });
});

// ---------------------------------------------------------------------------
// Retry action
// ---------------------------------------------------------------------------

describe("QueuePage — retry", () => {
    it("shows Retry button for retryable failed jobs", () => {
        const jobs = [makeJob({ status: "failed", retryable: true, error: "Network error" })];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
    });

    it("does not show Retry button for non-retryable failed jobs", () => {
        const jobs = [makeJob({ status: "failed", retryable: false, error: "Fatal error" })];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^history/i)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /retry/i })).not.toBeInTheDocument();
    });

    it("calls start_download when Retry is clicked", async () => {
        const startHandler = vi.fn(() => "new-job-id");
        setInvokeHandler("start_download", startHandler);
        const jobs = [
            makeJob({
                id: "j1",
                url: "https://example.com/retry-me",
                status: "failed",
                retryable: true,
                error: "Timeout",
            }),
        ];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        await userEvent.click(screen.getByRole("button", { name: /retry/i }));
        await waitFor(() => expect(startHandler).toHaveBeenCalled());
    });
});

// ---------------------------------------------------------------------------
// Clear All action
// ---------------------------------------------------------------------------

describe("QueuePage — Clear All", () => {
    it("shows Clear All button in the History section", () => {
        const jobs = [makeJob({ id: "j1", status: "completed" })];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^history/i)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /clear all/i })).toBeInTheDocument();
    });

    it("does not show Clear All button when history is empty", () => {
        const jobs = [makeJob({ id: "j1", status: "running", progress: 0.1 })];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        expect(screen.getByText(/^active/i)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /clear all/i })).not.toBeInTheDocument();
    });

    it("calls clear_completed_jobs when Clear All is clicked", async () => {
        const clearHandler = vi.fn(() => undefined);
        setInvokeHandler("clear_completed_jobs", clearHandler);
        const jobs = [
            makeJob({ id: "j1", status: "completed" }),
            makeJob({ id: "j2", status: "failed", error: "Error" }),
        ];
        render(<QueuePage />, { queryClient: seededClient(jobs) });
        await userEvent.click(screen.getByRole("button", { name: /clear all/i }));
        await waitFor(() => expect(clearHandler).toHaveBeenCalled());
    });
});
