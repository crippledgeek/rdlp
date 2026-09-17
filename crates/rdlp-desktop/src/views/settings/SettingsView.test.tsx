import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import { render } from "@/test/test-utils";
import { SettingsView } from "./SettingsView";
import { invokeTyped } from "@/api/invokeClient";
import { builtinNetworkStub, effectiveNetworkStub } from "@/test/effectiveNetworkStub";
import { effectiveNormalizeStub, loudnormPresetsStub } from "@/test/effectiveNormalizeStub";
import { appSettingsStub } from "@/test/appSettingsStub";
import userEvent from "@testing-library/user-event";

// Only `invokeTyped` is faked; `extractErrorMessage` stays real so the
// assertions exercise the unwrap the app actually ships.
vi.mock("@/api/invokeClient", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/api/invokeClient")>()),
    invokeTyped: vi.fn(),
}));

const invokeMock = vi.mocked(invokeTyped);

/** Reject exactly one command (with an `AppError`-shaped payload); resolve the other. */
function rejectOnly(
    failing: "settings" | "effective_network" | "builtin_network_defaults" | "effective_normalize" | "loudnorm_presets",
    message: string,
) {
    invokeMock.mockImplementation((cmd: string) => {
        if (cmd === failing) {
            return Promise.reject({ kind: "Internal", data: { message } });
        }
        if (cmd === "effective_network") return Promise.resolve(effectiveNetworkStub);
        if (cmd === "builtin_network_defaults") return Promise.resolve(builtinNetworkStub);
        if (cmd === "effective_normalize") return Promise.resolve(effectiveNormalizeStub);
        if (cmd === "loudnorm_presets") return Promise.resolve(loudnormPresetsStub);
        // `settings` never resolves in these tests: a settled `settings` would
        // need a full AppSettings fixture, and the failure branch under test
        // must win regardless of what the other query does.
        return new Promise(() => {});
    });
}

// The view gates the form on FIVE queries. A failure of any must surface as
// an error, not leave the pulse "Loading settings…" on screen forever — which
// is what a failed `settings` load did before #611's SettingsView change.
describe("SettingsView load errors", () => {
    beforeEach(() => {
        invokeMock.mockReset();
    });

    it("shows the error when the settings query rejects", async () => {
        rejectOnly("settings", "settings.json is unreadable");
        render(<SettingsView />);
        const alert = await screen.findByRole("alert");
        expect(alert).toHaveTextContent(/failed to load settings/i);
        expect(alert).toHaveTextContent("settings.json is unreadable");
        expect(screen.queryByText(/loading settings/i)).not.toBeInTheDocument();
    });

    it("shows the error when the effective-network query rejects", async () => {
        rejectOnly("effective_network", "engine config unavailable");
        render(<SettingsView />);
        const alert = await screen.findByRole("alert");
        expect(alert).toHaveTextContent(/failed to load settings/i);
        expect(alert).toHaveTextContent("engine config unavailable");
        expect(screen.queryByText(/loading settings/i)).not.toBeInTheDocument();
    });

    it("shows the error when the effective-normalize query rejects", async () => {
        rejectOnly("effective_normalize", "normalize defaults unavailable");
        render(<SettingsView />);
        const alert = await screen.findByRole("alert");
        expect(alert).toHaveTextContent("normalize defaults unavailable");
        expect(screen.queryByText(/loading settings/i)).not.toBeInTheDocument();
    });

    it("shows the error when the preset-catalogue query rejects", async () => {
        rejectOnly("loudnorm_presets", "preset catalogue unavailable");
        render(<SettingsView />);
        const alert = await screen.findByRole("alert");
        expect(alert).toHaveTextContent("preset catalogue unavailable");
        expect(screen.queryByText(/loading settings/i)).not.toBeInTheDocument();
    });

    it("shows the error when the built-in-defaults query rejects", async () => {
        rejectOnly("builtin_network_defaults", "defaults unavailable");
        render(<SettingsView />);
        const alert = await screen.findByRole("alert");
        expect(alert).toHaveTextContent("defaults unavailable");
        expect(screen.queryByText(/loading settings/i)).not.toBeInTheDocument();
    });
});

// Range validation has ONE owner — `rdlp_types::EffectiveNormalize::*_RANGE`,
// enforced by `AppSettings::validate_security` behind `update_settings` — so
// the view must NOT pre-screen values against a copied table. An out-of-range
// draft goes to the engine, and the engine's `OutOfRange` verdict is what the
// user sees (#611 review).
describe("SettingsView save path", () => {
    beforeEach(() => {
        invokeMock.mockReset();
    });

    it("forwards an out-of-range value to update_settings and shows the engine's verdict", async () => {
        const verdict = "loudnorm_target_lra: must be a finite number in 1..=50 LU";
        invokeMock.mockImplementation((cmd: string) => {
            switch (cmd) {
                case "settings":
                    // Out of range on purpose: the old client-side table blocked this.
                    return Promise.resolve({ ...appSettingsStub, normalize_audio: true, loudnorm: true, loudnorm_target_lra: 0 });
                case "effective_network":
                    return Promise.resolve(effectiveNetworkStub);
                case "builtin_network_defaults":
                    return Promise.resolve(builtinNetworkStub);
                case "effective_normalize":
                    return Promise.resolve(effectiveNormalizeStub);
                case "loudnorm_presets":
                    return Promise.resolve(loudnormPresetsStub);
                case "update_settings":
                    return Promise.reject({ kind: "InvalidInput", data: { field: "loudnorm_target_lra", message: verdict } });
                default:
                    return Promise.reject(new Error(`unexpected command ${cmd}`));
            }
        });
        const user = userEvent.setup();
        render(<SettingsView />);
        const save = await screen.findByRole("button", { name: /save settings/i });
        await user.click(save);

        const alert = await screen.findByRole("alert");
        expect(alert).toHaveTextContent(verdict);
        expect(invokeMock).toHaveBeenCalledWith("update_settings", expect.anything());
    });
});
