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
    it("renders correct initial state (single render)", () => {
        render(<DownloadPage />, { queryClient: seededClient() });
        expect(screen.getByPlaceholderText(/paste a video url/i)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^download$/i })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /choose format/i })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /^download$/i })).toBeDisabled();
    });

    it("shows validation error for non-URL input", () => {
        render(<DownloadPage />, { queryClient: seededClient() });
        const input = screen.getByPlaceholderText(/paste a video url/i);
        fireEvent.change(input, { target: { value: "not a url" } });
        fireEvent.click(screen.getByRole("button", { name: /^download$/i }));
        expect(screen.getByText(/enter a valid url/i)).toBeInTheDocument();
    });

    it("calls start_download with a valid URL and clears input", async () => {
        const startHandler = vi.fn(() => "job-123");
        setInvokeHandler("start_download", startHandler);
        render(<DownloadPage />, { queryClient: seededClient() });
        const input = screen.getByPlaceholderText(/paste a video url/i);
        fireEvent.change(input, { target: { value: "https://example.com/video" } });
        fireEvent.click(screen.getByRole("button", { name: /^download$/i }));
        await waitFor(() => {
            expect(startHandler).toHaveBeenCalled();
        });
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
        fireEvent.click(screen.getByRole("button", { name: /^download$/i }));
        await waitFor(() => {
            expect(screen.getByText(/failed to start download/i)).toBeInTheDocument();
        });
    });
});
