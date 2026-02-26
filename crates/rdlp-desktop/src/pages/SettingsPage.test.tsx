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
    embed_metadata: true,
    verbose: false,
    default_search_provider: null,
    normalize_audio: false,
    loudnorm: false,
    loudnorm_preset: null,
    loudnorm_target_i: null,
    loudnorm_target_tp: null,
    loudnorm_target_lra: null,
    loudnorm_dynamic: false,
    loudnorm_precompress: false,
    normalize_boost: false,
    normalize_boost_db: null,
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
});
