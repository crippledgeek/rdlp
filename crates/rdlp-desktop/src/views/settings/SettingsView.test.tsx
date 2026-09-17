import { describe, it, expect, vi, beforeEach } from "vitest";
import { screen } from "@testing-library/react";
import { render } from "@/test/test-utils";
import { SettingsView } from "./SettingsView";
import { invokeTyped } from "@/api/invokeClient";
import { builtinNetworkStub, effectiveNetworkStub } from "@/test/effectiveNetworkStub";
import { effectiveNormalizeStub, loudnormPresetsStub } from "@/test/effectiveNormalizeStub";

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
