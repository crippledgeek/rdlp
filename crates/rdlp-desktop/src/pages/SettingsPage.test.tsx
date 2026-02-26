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
});
