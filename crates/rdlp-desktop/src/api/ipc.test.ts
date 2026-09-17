// Compile-time contract of the typed command map (#611 review). Each
// `@ts-expect-error` line is a regression test: if the map ever stops
// rejecting that shape, `tsc` fails on the now-unused directive.

import { describe, it, expect, vi } from "vitest";
import type { IpcCommand, IpcResult, IpcStubTable } from "./ipc";
import { invokeTyped } from "./invokeClient";
import { appSettingsStub } from "../test/appSettingsStub";

vi.mock("./invokeClient", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./invokeClient")>()),
    invokeTyped: vi.fn(),
}));

// Never called at runtime (guarded below): the body exists for the type
// checker only.
function typeChecksOnly(): void {
    // @ts-expect-error TS2345 — a misspelt command is not a key of IpcCommands.
    void invokeTyped("settnigs");
    // @ts-expect-error TS2554 — a command with args cannot be called without them.
    void invokeTyped("effective_normalize");
    // @ts-expect-error TS2322 — the payload is typed per command.
    void invokeTyped("effective_normalize", { preset: "wrong" });
    // @ts-expect-error TS2554 — a void-args command takes no second argument.
    void invokeTyped("settings", {});
    // @ts-expect-error TS2322 — clear_completed_jobs returns a count, not void (download.rs).
    const _count: Promise<void> = invokeTyped("clear_completed_jobs");
    void _count;
    void ({
        // @ts-expect-error TS2561 — a misspelt key is rejected by the stub table.
        settnigs: () => Promise.resolve(appSettingsStub),
    } satisfies IpcStubTable);
}
if (Number.isNaN(0)) typeChecksOnly();

describe("IpcStubTable", () => {
    it("looks a stub up by command and rejects an unstubbed one", async () => {
        const stubs = { settings: () => Promise.resolve(appSettingsStub) } satisfies IpcStubTable;
        const lookup = <K extends IpcCommand>(cmd: K): IpcResult<K> => {
            const stub = (stubs as IpcStubTable)[cmd];
            return stub === undefined ? Promise.reject(new Error(`unexpected command ${cmd}`)) : stub();
        };
        await expect(lookup("settings")).resolves.toBe(appSettingsStub);
        await expect(lookup("queue")).rejects.toThrow("unexpected command queue");
    });
});
