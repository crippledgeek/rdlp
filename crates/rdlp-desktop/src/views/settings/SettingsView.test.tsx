import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import { render } from "@/test/test-utils";
import { SettingsView } from "./SettingsView";
import { invokeTyped } from "@/api/invokeClient";
import { networkDefaultsStub } from "@/test/effectiveNetworkStub";
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

type Command = "settings" | "network_defaults" | "effective_normalize" | "loudnorm_presets";

/** Resolve every gating command with its stub (`settings` with `appSettingsStub`). */
function resolveAll(cmd: string): Promise<unknown> {
    switch (cmd) {
        case "settings":
            return Promise.resolve(appSettingsStub);
        case "network_defaults":
            return Promise.resolve(networkDefaultsStub);
        case "effective_normalize":
            return Promise.resolve(effectiveNormalizeStub);
        case "loudnorm_presets":
            return Promise.resolve(loudnormPresetsStub);
        default:
            return Promise.reject(new Error(`unexpected command ${cmd}`));
    }
}

/** Reject exactly one command (with an `AppError`-shaped payload); resolve the others. */
function rejectOnly(failing: Command, message: string) {
    invokeMock.mockImplementation((cmd: string) => {
        if (cmd === failing) {
            return Promise.reject({ kind: "Internal", data: { message } });
        }
        return resolveAll(cmd);
    });
}

// The view gates the form on FOUR queries. A failure of any must surface as
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

    it("shows the error when the network-defaults query rejects", async () => {
        rejectOnly("network_defaults", "engine config unavailable");
        render(<SettingsView />);
        const alert = await screen.findByRole("alert");
        expect(alert).toHaveTextContent(/failed to load settings/i);
        expect(alert).toHaveTextContent("engine config unavailable");
        expect(screen.queryByText(/loading settings/i)).not.toBeInTheDocument();
    });

    // `effective_normalize` only runs once `settings` has resolved (it takes
    // the draft's preset), so `settings` resolves here and the normalize
    // query is the one that fails.
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
});

// The normalize payload is keyed by the DRAFT'S preset, and the draft does not
// exist until `settings` resolves. Fetching `effective_normalize(null)` before
// that and again with the real preset is a wasted round trip and, worse, a
// wrong payload on screen for one render (#611 review). The query is gated
// with `skipToken` until the settings arrive.
describe("SettingsView effective-normalize gating", () => {
    beforeEach(() => {
        invokeMock.mockReset();
    });

    it("does not invoke effective_normalize until settings resolve", async () => {
        invokeMock.mockImplementation((cmd: string) =>
            // `settings` never resolves: the sibling queries fire in the same
            // render, so once one of them has been invoked, an ungated
            // normalize query would have been invoked too.
            cmd === "settings" ? new Promise(() => {}) : resolveAll(cmd),
        );
        render(<SettingsView />);
        await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("loudnorm_presets"));
        await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("network_defaults"));
        expect(invokeMock).not.toHaveBeenCalledWith("effective_normalize", expect.anything());
        expect(screen.getByText(/loading settings/i)).toBeInTheDocument();
    });

    it("invokes effective_normalize exactly once, with the stored preset", async () => {
        invokeMock.mockImplementation((cmd: string) =>
            cmd === "settings"
                ? Promise.resolve({ ...appSettingsStub, loudnorm_preset: "loud" })
                : resolveAll(cmd),
        );
        render(<SettingsView />);
        await screen.findByRole("button", { name: /save settings/i });
        const normalizeCalls = invokeMock.mock.calls.filter(([cmd]) => cmd === "effective_normalize");
        expect(normalizeCalls).toEqual([["effective_normalize", { preset: "loud" }]]);
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
                case "update_settings":
                    return Promise.reject({ kind: "InvalidInput", data: { field: "loudnorm_target_lra", message: verdict } });
                default:
                    return resolveAll(cmd);
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
