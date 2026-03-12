import { render, screen, waitFor, fireEvent, createTestQueryClient } from "@/test/test-utils";
import { setInvokeHandler, clearInvokeHandlers } from "@/test/tauri-mock";
import { DownloadPage } from "./DownloadPage";
import { queryKeys } from "../query/queryKeys";
import type { AppSettings } from "../types";

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

/** Pre-seed the query cache so components render synchronously. */
function seededClient() {
    const qc = createTestQueryClient();
    qc.setQueryData(queryKeys.settings(), defaultSettings);
    qc.setQueryData(queryKeys.downloads.list(), []);
    return qc;
}

beforeEach(() => {
    setInvokeHandler("get_settings", () => defaultSettings);
    setInvokeHandler("get_queue", () => []);
});

afterEach(() => {
    clearInvokeHandlers();
});

describe("DownloadPage", () => {
    it("renders the URL input and Download button", () => {
        render(<DownloadPage />, { queryClient: seededClient() });
        expect(screen.getByPlaceholderText(/paste a video url/i)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /download/i })).toBeInTheDocument();
    });

    it("renders the Choose Format button", () => {
        render(<DownloadPage />, { queryClient: seededClient() });
        expect(screen.getByRole("button", { name: /choose format/i })).toBeInTheDocument();
    });

    it("shows validation error for non-URL input", async () => {
        render(<DownloadPage />, { queryClient: seededClient() });
        const input = screen.getByPlaceholderText(/paste a video url/i);
        fireEvent.change(input, { target: { value: "not a url" } });
        fireEvent.click(screen.getByRole("button", { name: /download/i }));
        expect(screen.getByText(/enter a valid url/i)).toBeInTheDocument();
    });

    it("calls start_download with a valid URL", async () => {
        const startHandler = vi.fn(() => "job-123");
        setInvokeHandler("start_download", startHandler);
        render(<DownloadPage />, { queryClient: seededClient() });
        const input = screen.getByPlaceholderText(/paste a video url/i);
        fireEvent.change(input, { target: { value: "https://example.com/video" } });
        fireEvent.click(screen.getByRole("button", { name: /download/i }));
        await waitFor(() => {
            expect(startHandler).toHaveBeenCalled();
        });
    });

    it("clears the input after successful download start", async () => {
        setInvokeHandler("start_download", () => "job-123");
        render(<DownloadPage />, { queryClient: seededClient() });
        const input = screen.getByPlaceholderText(/paste a video url/i);
        fireEvent.change(input, { target: { value: "https://example.com/video" } });
        fireEvent.click(screen.getByRole("button", { name: /download/i }));
        await waitFor(() => {
            expect(input).toHaveValue("");
        });
    });

    it("shows error message when start_download fails", async () => {
        setInvokeHandler("start_download", () => {
            throw { kind: "InvalidInput", data: { message: "URL blocked by security policy" } };
        });
        render(<DownloadPage />, { queryClient: seededClient() });
        const input = screen.getByPlaceholderText(/paste a video url/i);
        fireEvent.change(input, { target: { value: "https://evil.local/video" } });
        fireEvent.click(screen.getByRole("button", { name: /download/i }));
        await waitFor(() => {
            expect(screen.getByText(/failed to start download/i)).toBeInTheDocument();
        });
    });

    it("disables Download button when input is empty", () => {
        render(<DownloadPage />, { queryClient: seededClient() });
        expect(screen.getByRole("button", { name: /download/i })).toBeDisabled();
    });
});
