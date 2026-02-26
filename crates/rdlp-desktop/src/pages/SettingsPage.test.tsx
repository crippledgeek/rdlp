import { render, screen, waitFor } from "@/test/test-utils";
import userEvent from "@testing-library/user-event";
import { setInvokeHandler, clearInvokeHandlers } from "@/test/tauri-mock";
import { SettingsPage } from "./SettingsPage";
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

beforeEach(() => {
    setInvokeHandler("get_settings", () => defaultSettings);
    setInvokeHandler("get_search_providers", () => [
        { name: "redtube", display_name: "RedTube" },
    ]);
    setInvokeHandler("update_settings", () => undefined);
});

afterEach(() => {
    clearInvokeHandlers();
});

describe("SettingsPage", () => {
    it("shows loading state initially", () => {
        render(<SettingsPage />);
        expect(screen.getByText(/loading settings/i)).toBeInTheDocument();
    });

    it("renders the settings form after loading", async () => {
        render(<SettingsPage />);
        await waitFor(() => {
            expect(screen.getByRole("heading", { name: /settings/i })).toBeInTheDocument();
        });
    });

    it("displays the output directory value", async () => {
        render(<SettingsPage />);
        await waitFor(() => {
            expect(screen.getByDisplayValue("/home/user/Videos")).toBeInTheDocument();
        });
    });

    it("renders the Save Settings button", async () => {
        render(<SettingsPage />);
        await waitFor(() => {
            expect(screen.getByRole("button", { name: /save settings/i })).toBeInTheDocument();
        });
    });

    it("calls update_settings invoke when Save is clicked", async () => {
        const updateHandler = vi.fn(() => undefined);
        setInvokeHandler("update_settings", updateHandler);

        render(<SettingsPage />);
        await waitFor(() => screen.getByRole("button", { name: /save settings/i }));
        await userEvent.click(screen.getByRole("button", { name: /save settings/i }));

        expect(updateHandler).toHaveBeenCalledTimes(1);
    });

    it("shows error message when save fails", async () => {
        // invokeTyped wraps errors into { code, message, details }.
        // SettingsPage.handleSave calls String(err) on non-Error rejects,
        // so throw an AppError-shaped object whose message invokeClient can extract.
        setInvokeHandler("update_settings", () => {
            throw { kind: "SaveFailed", data: { message: "Backend error" } };
        });

        render(<SettingsPage />);
        await waitFor(() => screen.getByRole("button", { name: /save settings/i }));
        await userEvent.click(screen.getByRole("button", { name: /save settings/i }));

        await waitFor(() => {
            // invokeTyped extracts the message from the AppError data, but handleSave
            // then calls String() on the InvokeError object → check the error is visible
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
    });

    it("embed thumbnails checkbox reflects current setting", async () => {
        render(<SettingsPage />);
        await waitFor(() => screen.getByRole("heading", { name: /settings/i }));

        // embed_thumbnail is true in defaultSettings → checkbox should be checked
        const checkboxes = screen.getAllByRole("checkbox");
        const thumbnailCheckbox = checkboxes[0];
        expect(thumbnailCheckbox).toBeChecked();
    });

    it("verbose logging checkbox unchecked by default", async () => {
        render(<SettingsPage />);
        await waitFor(() => screen.getByRole("heading", { name: /settings/i }));

        // verbose is false in defaultSettings
        const verboseLabel = screen.getByText(/verbose logging/i);
        const checkbox = verboseLabel.closest("div")?.querySelector('button[role="checkbox"]');
        expect(checkbox).toHaveAttribute("aria-checked", "false");
    });

    it("renders Browse button for output directory", async () => {
        render(<SettingsPage />);
        await waitFor(() => {
            expect(screen.getByRole("button", { name: /browse/i })).toBeInTheDocument();
        });
    });

    it("shows normalize audio checkbox unchecked by default", async () => {
        render(<SettingsPage />);
        await waitFor(() => screen.getByText(/normalize audio/i));
        const label = screen.getByText(/normalize audio/i);
        const checkbox = label.closest("div")?.querySelector('button[role="checkbox"]');
        expect(checkbox).toHaveAttribute("aria-checked", "false");
    });

    it("reveals mode toggle and boost fallback when normalize audio is checked", async () => {
        render(<SettingsPage />);
        await waitFor(() => screen.getByText(/normalize audio/i));
        // Click the checkbox button directly to avoid double-fire from label+div onClick
        const normalizeLabel = screen.getByText(/normalize audio/i);
        const checkbox = normalizeLabel.closest("div")?.querySelector('button[role="checkbox"]');
        await userEvent.click(checkbox!);
        await waitFor(() => {
            expect(screen.getByText(/ebu r128 loudnorm/i)).toBeInTheDocument();
        });
        expect(screen.getByText(/boost fallback/i)).toBeInTheDocument();
    });

    it("reveals preset select and advanced options when loudnorm mode is active", async () => {
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            loudnorm: true,
        }));
        render(<SettingsPage />);
        await waitFor(() => screen.getByText(/dynamic mode/i));
        expect(screen.getByText(/dynamic mode/i)).toBeInTheDocument();
        expect(screen.getByText(/precompress/i)).toBeInTheDocument();
        // Preset label is present (using heading role to disambiguate from select items)
        expect(screen.getByText("Preset")).toBeInTheDocument();
    });

    it("reveals boost gain input when boost fallback is checked", async () => {
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            loudnorm: true,
            normalize_boost: true,
        }));
        render(<SettingsPage />);
        await waitFor(() => screen.getByPlaceholderText("12.0"));
        expect(screen.getByPlaceholderText("12.0")).toBeInTheDocument();
    });

    // -----------------------------------------------------------------------
    // A. Range validation errors on save
    // -----------------------------------------------------------------------

    it("shows error when loudnorm_target_i is out of range (above 0)", async () => {
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            loudnorm: true,
        }));
        render(<SettingsPage />);
        await waitFor(() => screen.getByLabelText(/loudness target/i));

        // Enter an out-of-range value: 5 is > 0, valid range is -70..0
        const input = screen.getByLabelText(/loudness target/i);
        await userEvent.clear(input);
        await userEvent.type(input, "5");

        await userEvent.click(screen.getByRole("button", { name: /save settings/i }));
        await waitFor(() => {
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
        expect(screen.getByRole("alert")).toHaveTextContent(/loudness target/i);
    });

    it("shows error when loudnorm_target_tp is out of range (above 0)", async () => {
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            loudnorm: true,
        }));
        render(<SettingsPage />);
        await waitFor(() => screen.getByLabelText(/true peak limit/i));

        // Enter 1.0 — above the 0 maximum for true peak limit
        const input = screen.getByLabelText(/true peak limit/i);
        await userEvent.clear(input);
        await userEvent.type(input, "1");

        await userEvent.click(screen.getByRole("button", { name: /save settings/i }));
        await waitFor(() => {
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
        expect(screen.getByRole("alert")).toHaveTextContent(/true peak/i);
    });

    it("shows error when normalize_boost_db is out of range (above 30)", async () => {
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            normalize_boost: true,
        }));
        render(<SettingsPage />);
        await waitFor(() => screen.getByPlaceholderText("12.0"));

        // Enter 50 — above the 30 maximum for boost gain
        const input = screen.getByPlaceholderText("12.0");
        await userEvent.clear(input);
        await userEvent.type(input, "50");

        await userEvent.click(screen.getByRole("button", { name: /save settings/i }));
        await waitFor(() => {
            expect(screen.getByRole("alert")).toBeInTheDocument();
        });
        expect(screen.getByRole("alert")).toHaveTextContent(/boost gain/i);
    });

    // -----------------------------------------------------------------------
    // B. Save payload with normalization settings
    // -----------------------------------------------------------------------

    it("calls update_settings with correct normalization payload for loudnorm broadcast preset", async () => {
        const updateHandler = vi.fn((_args?: Record<string, unknown>) => undefined);
        setInvokeHandler("update_settings", updateHandler);
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: "broadcast",
        }));

        render(<SettingsPage />);
        await waitFor(() => screen.getByRole("button", { name: /save settings/i }));
        await userEvent.click(screen.getByRole("button", { name: /save settings/i }));

        expect(updateHandler).toHaveBeenCalledTimes(1);
        // updateSettings calls invoke("update_settings", { settings }) so the mock
        // handler receives { settings: AppSettings } as its first argument.
        const args = updateHandler.mock.calls[0][0] as { settings: AppSettings };
        expect(args.settings.normalize_audio).toBe(true);
        expect(args.settings.loudnorm).toBe(true);
        expect(args.settings.loudnorm_preset).toBe("broadcast");
    });

    it("calls update_settings with normalize_audio false when unchecked", async () => {
        const updateHandler = vi.fn((_args?: Record<string, unknown>) => undefined);
        setInvokeHandler("update_settings", updateHandler);

        render(<SettingsPage />);
        await waitFor(() => screen.getByRole("button", { name: /save settings/i }));
        await userEvent.click(screen.getByRole("button", { name: /save settings/i }));

        expect(updateHandler).toHaveBeenCalledTimes(1);
        const args = updateHandler.mock.calls[0][0] as { settings: AppSettings };
        expect(args.settings.normalize_audio).toBe(false);
    });

    // -----------------------------------------------------------------------
    // C. Visibility cascading
    // -----------------------------------------------------------------------

    it("loudnorm mode toggle not visible when normalize_audio is unchecked", async () => {
        render(<SettingsPage />);
        await waitFor(() => screen.getByRole("heading", { name: /settings/i }));
        // normalize_audio is false in defaultSettings, so loudnorm toggle should be hidden
        expect(screen.queryByText(/ebu r128 loudnorm/i)).not.toBeInTheDocument();
    });

    it("preset select and custom targets not visible when normalize_audio is checked but mode is Peak", async () => {
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            loudnorm: false,
        }));
        render(<SettingsPage />);
        await waitFor(() => screen.getByText(/ebu r128 loudnorm/i));
        // Peak mode: loudnorm-specific options should not appear
        expect(screen.queryByText(/dynamic mode/i)).not.toBeInTheDocument();
        expect(screen.queryByText(/precompress/i)).not.toBeInTheDocument();
        expect(screen.queryByText("Preset")).not.toBeInTheDocument();
    });

    it("boost dB input not visible when boost fallback is unchecked", async () => {
        setInvokeHandler("get_settings", () => ({
            ...defaultSettings,
            normalize_audio: true,
            normalize_boost: false,
        }));
        render(<SettingsPage />);
        await waitFor(() => screen.getByText(/boost fallback/i));
        // normalize_boost is false → boost gain input should not appear
        expect(screen.queryByPlaceholderText("12.0")).not.toBeInTheDocument();
    });
});
