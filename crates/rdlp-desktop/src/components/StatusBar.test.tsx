import { render, screen, act } from "@/test/test-utils";
import userEvent from "@testing-library/user-event";
import { setInvokeHandler, clearInvokeHandlers } from "@/test/tauri-mock";
import { StatusBar } from "./StatusBar";
import { resetSearchParams } from "../stores/searchParamsStore";

beforeEach(() => {
    resetSearchParams();
    setInvokeHandler("get_search_providers", () => []);
    setInvokeHandler("get_queue", () => []);
});

afterEach(() => {
    clearInvokeHandlers();
    act(() => resetSearchParams());
});

function renderStatusBar(props?: {
    viewMode?: "list" | "grid";
    onViewModeChange?: (mode: "list" | "grid") => void;
    onSwitchToQueue?: () => void;
}) {
    return render(
        <StatusBar
            viewMode={props?.viewMode ?? "list"}
            onViewModeChange={props?.onViewModeChange ?? vi.fn()}
            onSwitchToQueue={props?.onSwitchToQueue ?? vi.fn()}
        />,
    );
}

describe("StatusBar", () => {
    it("renders 'Ready' in idle state", () => {
        renderStatusBar();
        expect(screen.getByText("Ready")).toBeInTheDocument();
    });

    it("renders List view button", () => {
        renderStatusBar();
        expect(screen.getByRole("button", { name: /list view/i })).toBeInTheDocument();
    });

    it("renders Grid view button", () => {
        renderStatusBar();
        expect(screen.getByRole("button", { name: /grid view/i })).toBeInTheDocument();
    });

    it("calls onViewModeChange with 'grid' when Grid view is clicked", async () => {
        const onViewModeChange = vi.fn();
        renderStatusBar({ onViewModeChange });
        await userEvent.click(screen.getByRole("button", { name: /grid view/i }));
        expect(onViewModeChange).toHaveBeenCalledWith("grid");
    });

    it("calls onViewModeChange with 'list' when List view is clicked", async () => {
        const onViewModeChange = vi.fn();
        renderStatusBar({ viewMode: "grid", onViewModeChange });
        await userEvent.click(screen.getByRole("button", { name: /list view/i }));
        expect(onViewModeChange).toHaveBeenCalledWith("list");
    });

    it("does not render a queue link when no active downloads", () => {
        renderStatusBar();
        // The center text button linking to the queue only appears with active jobs
        expect(screen.queryByTitle(/switch to queue/i)).not.toBeInTheDocument();
    });

    it("calls onSwitchToQueue when active download link is clicked", async () => {
        const onSwitchToQueue = vi.fn();

        // Return an active job so the center button appears
        setInvokeHandler("get_queue", () => [
            {
                id: "job-1",
                url: "https://example.com/video",
                title: "Video",
                status: "running",
                progress: 0.5,
                speed: "2 MB/s",
                eta: "5s",
                error: null,
                retryable: false,
                started_at: null,
                completed_at: null,
                output_path: null,
                statusMessage: null,
            },
        ]);

        renderStatusBar({ onSwitchToQueue });

        // Wait for queue to load and the center button to appear
        const link = await screen.findByTitle(/switch to queue/i);
        await userEvent.click(link);
        expect(onSwitchToQueue).toHaveBeenCalledTimes(1);
    });
});
