import { render, screen, waitFor } from "@/test/test-utils";
import userEvent from "@testing-library/user-event";
import { setInvokeHandler, clearInvokeHandlers } from "@/test/tauri-mock";
import { DownloadPage } from "./DownloadPage";
import type { AppSettings } from "../types";

const defaultSettings: AppSettings = {
    output_dir: "/home/user/Videos",
    default_remux: null,
    default_extract_audio: null,
    default_subtitle_format: null,
    default_subtitle_langs: [],
    embed_thumbnail: true,
    embed_metadata: true,
    verbose: false,
    default_search_provider: null,
};

beforeEach(() => {
    setInvokeHandler("get_settings", () => defaultSettings);
    setInvokeHandler("get_queue", () => []);
});

afterEach(() => {
    clearInvokeHandlers();
});

describe("DownloadPage", () => {
    it("renders the URL input and Download button", async () => {
        render(<DownloadPage />);
        expect(screen.getByPlaceholderText(/paste a video url/i)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /download/i })).toBeInTheDocument();
    });

    it("renders the Choose Format button", () => {
        render(<DownloadPage />);
        expect(screen.getByRole("button", { name: /choose format/i })).toBeInTheDocument();
    });

    it("shows validation error for non-URL input", async () => {
        const user = userEvent.setup();
        render(<DownloadPage />);
        const input = screen.getByPlaceholderText(/paste a video url/i);
        await user.type(input, "not a url");
        await user.click(screen.getByRole("button", { name: /download/i }));
        await waitFor(() => {
            expect(screen.getByText(/enter a valid url/i)).toBeInTheDocument();
        });
    });

    it("calls start_download with a valid URL", async () => {
        const startHandler = vi.fn(() => "job-123");
        setInvokeHandler("start_download", startHandler);
        const user = userEvent.setup();
        render(<DownloadPage />);
        const input = screen.getByPlaceholderText(/paste a video url/i);
        await user.type(input, "https://example.com/video");
        await user.click(screen.getByRole("button", { name: /download/i }));
        await waitFor(() => {
            expect(startHandler).toHaveBeenCalled();
        });
    });

    it("clears the input after successful download start", async () => {
        setInvokeHandler("start_download", () => "job-123");
        const user = userEvent.setup();
        render(<DownloadPage />);
        const input = screen.getByPlaceholderText(/paste a video url/i);
        await user.type(input, "https://example.com/video");
        await user.click(screen.getByRole("button", { name: /download/i }));
        await waitFor(() => {
            expect(input).toHaveValue("");
        });
    });

    it("shows error message when start_download fails", async () => {
        setInvokeHandler("start_download", () => {
            throw { kind: "InvalidInput", data: { message: "URL blocked by security policy" } };
        });
        const user = userEvent.setup();
        render(<DownloadPage />);
        const input = screen.getByPlaceholderText(/paste a video url/i);
        await user.type(input, "https://evil.local/video");
        await user.click(screen.getByRole("button", { name: /download/i }));
        await waitFor(() => {
            expect(screen.getByText(/failed to start download/i)).toBeInTheDocument();
        });
    });

    it("disables Download button when input is empty", () => {
        render(<DownloadPage />);
        expect(screen.getByRole("button", { name: /download/i })).toBeDisabled();
    });
});
