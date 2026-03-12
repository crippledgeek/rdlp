import { render, screen, waitFor, fireEvent, createTestQueryClient } from "@/test/test-utils";
import { setInvokeHandler, clearInvokeHandlers } from "@/test/tauri-mock";
import { SettingsPage } from "./SettingsPage";
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

const mockProviders = [{ name: "redtube", display_name: "RedTube" }];

/** Pre-seed the query cache so SettingsPage renders synchronously. */
function seededClient(settings: AppSettings = defaultSettings) {
    const qc = createTestQueryClient();
    qc.setQueryData(queryKeys.settings(), settings);
    qc.setQueryData(queryKeys.providers(), mockProviders);
    return qc;
}

beforeEach(() => {
    setInvokeHandler("get_settings", () => defaultSettings);
    setInvokeHandler("get_search_providers", () => mockProviders);
    setInvokeHandler("update_settings", () => undefined);
});

afterEach(() => {
    clearInvokeHandlers();
});

describe("SettingsPage", () => {
    it("shows loading state initially", () => {
        // Do NOT pre-seed — test the loading skeleton
        render(<SettingsPage />);
        expect(screen.getByText(/loading settings/i)).toBeInTheDocument();
    });

    it("renders correctly after loading", () => {
        render(<SettingsPage />, { queryClient: seededClient() });
        expect(screen.getByRole("heading", { name: /settings/i })).toBeInTheDocument();
        expect(screen.getByDisplayValue("/home/user/Videos")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /save settings/i })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /browse/i })).toBeInTheDocument();
        // Checkbox defaults
        const checkboxes = screen.getAllByRole("checkbox");
        expect(checkboxes[0]).toBeChecked(); // embed thumbnail
        const verboseLabel = screen.getByText(/verbose logging/i);
        const checkbox = verboseLabel.closest("div")?.querySelector('button[role="checkbox"]');
        expect(checkbox).toHaveAttribute("aria-checked", "false");
    });

    it("calls update_settings invoke when Save is clicked", () => {
        const updateHandler = vi.fn(() => undefined);
        setInvokeHandler("update_settings", updateHandler);

        render(<SettingsPage />, { queryClient: seededClient() });
        fireEvent.click(screen.getByRole("button", { name: /save settings/i }));

        expect(updateHandler).toHaveBeenCalledTimes(1);
    });

    it("shows error message when save fails", async () => {
        setInvokeHandler("update_settings", () => {
            throw { kind: "SaveFailed", data: { message: "Backend error" } };
        });

        render(<SettingsPage />, { queryClient: seededClient() });
        fireEvent.click(screen.getByRole("button", { name: /save settings/i }));

        await waitFor(() => {
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
    });

    // Validation range errors are tested as pure functions in settingsValidation.test.ts.
    // This single integration test verifies the wiring from validation → UI error display.
    it("shows validation error on save when a field is out of range", async () => {
        const settings = { ...defaultSettings, normalize_audio: true, loudnorm: true };
        render(<SettingsPage />, { queryClient: seededClient(settings) });

        fireEvent.change(screen.getByLabelText(/loudness target/i), { target: { value: "5" } });
        fireEvent.click(screen.getByRole("button", { name: /save settings/i }));
        await waitFor(() => {
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
        expect(screen.getByRole("alert")).toHaveTextContent(/loudness target/i);
    });

    it("calls update_settings with correct normalization payload", () => {
        const updateHandler = vi.fn((_args?: Record<string, unknown>) => undefined);
        setInvokeHandler("update_settings", updateHandler);
        const settings = { ...defaultSettings, normalize_audio: true, loudnorm: true, loudnorm_preset: "broadcast" };

        render(<SettingsPage />, { queryClient: seededClient(settings) });
        fireEvent.click(screen.getByRole("button", { name: /save settings/i }));

        expect(updateHandler).toHaveBeenCalledTimes(1);
        const args = updateHandler.mock.calls[0][0] as { settings: AppSettings };
        expect(args.settings.normalize_audio).toBe(true);
        expect(args.settings.loudnorm).toBe(true);
        expect(args.settings.loudnorm_preset).toBe("broadcast");
    });

    it("calls update_settings with normalize_audio false when unchecked", () => {
        const updateHandler = vi.fn((_args?: Record<string, unknown>) => undefined);
        setInvokeHandler("update_settings", updateHandler);

        render(<SettingsPage />, { queryClient: seededClient() });
        fireEvent.click(screen.getByRole("button", { name: /save settings/i }));

        expect(updateHandler).toHaveBeenCalledTimes(1);
        const args = updateHandler.mock.calls[0][0] as { settings: AppSettings };
        expect(args.settings.normalize_audio).toBe(false);
    });
});
