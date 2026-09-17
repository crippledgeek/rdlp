// The effective-network query: the base configuration is loaded once per
// process, so its resolved values never go stale while the app runs (#611).

import { describe, it, expect } from "vitest";
import { effectiveNetworkQueryOptions } from "./effectiveNetwork";
import { queryKeys } from "../query/queryKeys";

describe("effectiveNetworkQueryOptions", () => {
    it("uses the centralized query key", () => {
        expect(effectiveNetworkQueryOptions().queryKey).toEqual(queryKeys.effectiveNetwork());
    });

    it("never goes stale — the base configuration is read once per process", () => {
        expect(effectiveNetworkQueryOptions().staleTime).toBe(Infinity);
    });

    it("has a real queryFn (not lazily gated)", () => {
        expect(typeof effectiveNetworkQueryOptions().queryFn).toBe("function");
    });
});
