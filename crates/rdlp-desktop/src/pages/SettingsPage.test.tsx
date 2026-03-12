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

    it("renders correct initial state (single render)", () => {
        render(<SettingsPage />, { queryClient: seededClient() });
        expect(screen.getByRole("heading", { name: /settings/i })).toBeInTheDocument();
        expect(screen.getByDisplayValue("/home/user/Videos")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /save settings/i })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /browse/i })).toBeInTheDocument();
    });

    it("calls update_settings invoke when Save is clicked", async () => {
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

    it("checkbox defaults reflect settings (single render)", () => {
        render(<SettingsPage />, { queryClient: seededClient() });
        // Embed thumbnail checked by default
        const checkboxes = screen.getAllByRole("checkbox");
        expect(checkboxes[0]).toBeChecked();
        // Verbose logging unchecked
        const verboseLabel = screen.getByText(/verbose logging/i);
        const verboseCheckbox = verboseLabel.closest("div")?.querySelector('button[role="checkbox"]');
        expect(verboseCheckbox).toHaveAttribute("aria-checked", "false");
        // Normalize audio unchecked
        const normLabel = screen.getByText(/normalize audio/i);
        const normCheckbox = normLabel.closest("div")?.querySelector('button[role="checkbox"]');
        expect(normCheckbox).toHaveAttribute("aria-checked", "false");
    });

    it("reveals mode toggle and boost fallback when normalize audio is checked", async () => {
        render(<SettingsPage />, { queryClient: seededClient() });
        const normalizeLabel = screen.getByText(/normalize audio/i);
        const checkbox = normalizeLabel.closest("div")?.querySelector('button[role="checkbox"]');
        fireEvent.click(checkbox!);
        await waitFor(() => {
            expect(screen.getByText(/ebu r128 loudnorm/i)).toBeInTheDocument();
        });
        expect(screen.getByText(/boost fallback/i)).toBeInTheDocument();
    });

    it("reveals preset select and advanced options when loudnorm mode is active", () => {
        const settings = { ...defaultSettings, normalize_audio: true, loudnorm: true };
        render(<SettingsPage />, { queryClient: seededClient(settings) });
        expect(screen.getByText(/dynamic mode/i)).toBeInTheDocument();
        expect(screen.getByText(/precompress/i)).toBeInTheDocument();
        expect(screen.getByText("Preset")).toBeInTheDocument();
    });

    it("reveals boost gain input when boost fallback is checked", () => {
        const settings = { ...defaultSettings, normalize_audio: true, loudnorm: true, normalize_boost: true };
        render(<SettingsPage />, { queryClient: seededClient(settings) });
        expect(screen.getByPlaceholderText("12.0")).toBeInTheDocument();
    });

    // -----------------------------------------------------------------------
    // A. Range validation errors on save
    // -----------------------------------------------------------------------

    it.each([
        ["loudness target (above 0)", { normalize_audio: true, loudnorm: true }, /loudness target/i, "5", /loudness target/i],
        ["true peak (above 0)", { normalize_audio: true, loudnorm: true }, /true peak limit/i, "1", /true peak/i],
    ] as const)("shows error when %s is out of range", async (_label, settingsOverrides, labelPattern, value, errorPattern) => {
        const settings = { ...defaultSettings, ...settingsOverrides };
        render(<SettingsPage />, { queryClient: seededClient(settings) });
        fireEvent.change(screen.getByLabelText(labelPattern), { target: { value } });
        fireEvent.click(screen.getByRole("button", { name: /save settings/i }));
        await waitFor(() => {
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
        expect(screen.getByRole("alert")).toHaveTextContent(errorPattern);
    });

    it("shows error when normalize_boost_db is out of range (above 30)", async () => {
        const settings = { ...defaultSettings, normalize_audio: true, normalize_boost: true };
        render(<SettingsPage />, { queryClient: seededClient(settings) });
        fireEvent.change(screen.getByPlaceholderText("12.0"), { target: { value: "50" } });
        fireEvent.click(screen.getByRole("button", { name: /save settings/i }));
        await waitFor(() => {
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
        expect(screen.getByRole("alert")).toHaveTextContent(/boost gain/i);
    });

    // -----------------------------------------------------------------------
    // B. Save payload with normalization settings
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // C. Visibility cascading
    // -----------------------------------------------------------------------

    it("hides normalization sub-options based on settings state", () => {
        // Default: normalize_audio off → no loudnorm toggle
        const { unmount } = render(<SettingsPage />, { queryClient: seededClient() });
        expect(screen.queryByText(/ebu r128 loudnorm/i)).not.toBeInTheDocument();
        unmount();

        // Peak mode: no loudnorm sub-options
        const settings1 = { ...defaultSettings, normalize_audio: true, loudnorm: false };
        const { unmount: unmount2 } = render(<SettingsPage />, { queryClient: seededClient(settings1) });
        expect(screen.queryByText(/dynamic mode/i)).not.toBeInTheDocument();
        expect(screen.queryByText(/precompress/i)).not.toBeInTheDocument();
        expect(screen.queryByText("Preset")).not.toBeInTheDocument();
        unmount2();

        // Boost unchecked: no dB input
        const settings2 = { ...defaultSettings, normalize_audio: true, normalize_boost: false };
        render(<SettingsPage />, { queryClient: seededClient(settings2) });
        expect(screen.queryByPlaceholderText("12.0")).not.toBeInTheDocument();
    });
});
