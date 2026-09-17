// The network-defaults query: the base configuration is loaded once per
// process and the built-in defaults and ranges are constants, so the payload
// never goes stale while the app runs (#611).

import { describe, it, expect, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { networkDefaultsQueryOptions } from "./effectiveNetwork";
import { queryKeys } from "../query/queryKeys";
import { invokeTyped } from "./invokeClient";
import { networkDefaultsStub } from "../test/effectiveNetworkStub";

vi.mock("./invokeClient", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./invokeClient")>()),
    invokeTyped: vi.fn(),
}));

describe("networkDefaultsQueryOptions", () => {
    it("uses the centralized query key", () => {
        expect(networkDefaultsQueryOptions().queryKey).toEqual(queryKeys.networkDefaults());
    });

    it("never goes stale — the base configuration is read once per process", () => {
        expect(networkDefaultsQueryOptions().staleTime).toBe(Infinity);
    });

    it("invokes the one network_defaults command", async () => {
        const invokeMock = vi.mocked(invokeTyped);
        invokeMock.mockResolvedValueOnce(networkDefaultsStub);
        await new QueryClient().fetchQuery(networkDefaultsQueryOptions());
        expect(invokeMock).toHaveBeenCalledTimes(1);
        expect(invokeMock).toHaveBeenCalledWith("network_defaults");
    });
});
