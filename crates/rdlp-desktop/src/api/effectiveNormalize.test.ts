// The effective-normalize query: keyed by preset, never stale, and it keeps
// the previous preset's payload while the next one loads (#611).

import { describe, it, expect, vi } from "vitest";
import { QueryClient, keepPreviousData, skipToken } from "@tanstack/react-query";
import { effectiveNormalizeQueryOptions, loudnormPresetsQueryOptions } from "./effectiveNormalize";
import { queryKeys } from "../query/queryKeys";
import { invokeTyped } from "./invokeClient";

vi.mock("./invokeClient", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./invokeClient")>()),
    invokeTyped: vi.fn(),
}));

describe("effectiveNormalizeQueryOptions", () => {
    it("uses the centralized query key, keyed by the preset", () => {
        expect(effectiveNormalizeQueryOptions("loud").queryKey).toEqual(queryKeys.effectiveNormalize("loud"));
        expect(effectiveNormalizeQueryOptions(null).queryKey).toEqual(queryKeys.effectiveNormalize(null));
    });

    it("never goes stale — the base configuration is read once per process", () => {
        expect(effectiveNormalizeQueryOptions(null).staleTime).toBe(Infinity);
    });

    it("keeps the previous preset's payload while the next one loads", () => {
        expect(effectiveNormalizeQueryOptions(null).placeholderData).toBe(keepPreviousData);
    });

    // No draft yet (`undefined`) gates the query; the stored "inherit" (`null`)
    // does not — the two must not collapse into one, or the view would either
    // never fetch for an inheriting draft or fetch before the draft exists.
    it("is gated with skipToken until the draft exists, and fetches for a null (inherit) preset", () => {
        expect(effectiveNormalizeQueryOptions(undefined).queryFn).toBe(skipToken);
        expect(effectiveNormalizeQueryOptions(undefined).queryKey).toEqual(queryKeys.effectiveNormalize(null));
        expect(typeof effectiveNormalizeQueryOptions(null).queryFn).toBe("function");
    });

    // The command takes the preset as its argument; a query that forgot to
    // pass it would resolve every preset to the base configuration's.
    it("passes the preset to the effective_normalize command", async () => {
        const invokeMock = vi.mocked(invokeTyped);
        invokeMock.mockResolvedValueOnce({});
        await new QueryClient().fetchQuery(effectiveNormalizeQueryOptions("broadcast"));
        expect(invokeMock).toHaveBeenCalledWith("effective_normalize", { preset: "broadcast" });
    });
});

describe("loudnormPresetsQueryOptions", () => {
    it("uses its own centralized query key", () => {
        expect(loudnormPresetsQueryOptions().queryKey).toEqual(queryKeys.loudnormPresets());
    });

    it("never goes stale — compile-time constants", () => {
        expect(loudnormPresetsQueryOptions().staleTime).toBe(Infinity);
    });

    it("invokes the loudnorm_presets command", async () => {
        const invokeMock = vi.mocked(invokeTyped);
        invokeMock.mockResolvedValueOnce([]);
        await new QueryClient().fetchQuery(loudnormPresetsQueryOptions());
        expect(invokeMock).toHaveBeenCalledWith("loudnorm_presets");
    });
});
