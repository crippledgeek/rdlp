// Tests for the skipToken-based lazy query factories.
// Regression guard for bug H5: enabled-override on queryOptions() spread
// silently no-ops under React 19 / TanStack Query v5. The factories MUST
// gate via `skipToken` in the queryFn slot instead.

import { describe, it, expect } from "vitest";
import { skipToken } from "@tanstack/react-query";
import { codecsQueryOptions } from "./codecs";
import { audioCodecsQueryOptions } from "./audioCodecs";

describe("codecsQueryOptions", () => {
    it("uses skipToken as queryFn when ready=false (regression: was using enabled override)", () => {
        const opts = codecsQueryOptions(false);
        // The queryFn must be skipToken itself, not a real function.
        // If this assertion fails, the `enabled` override was re-introduced.
        expect(opts.queryFn).toBe(skipToken);
    });

    it("uses a real function as queryFn when ready=true", () => {
        const opts = codecsQueryOptions(true);
        expect(typeof opts.queryFn).toBe("function");
        expect(opts.queryFn).not.toBe(skipToken);
    });

    it("uses the same queryKey regardless of ready flag", () => {
        const whenReady = codecsQueryOptions(true);
        const whenNotReady = codecsQueryOptions(false);
        expect(whenReady.queryKey).toEqual(whenNotReady.queryKey);
    });
});

describe("audioCodecsQueryOptions", () => {
    it("uses skipToken as queryFn when ready=false", () => {
        const opts = audioCodecsQueryOptions("mp4", false);
        expect(opts.queryFn).toBe(skipToken);
    });

    it("uses a real function as queryFn when ready=true", () => {
        const opts = audioCodecsQueryOptions("mp4", true);
        expect(typeof opts.queryFn).toBe("function");
        expect(opts.queryFn).not.toBe(skipToken);
    });

    it("includes container in the queryKey", () => {
        const optsWithContainer = audioCodecsQueryOptions("webm", true);
        const optsNullContainer = audioCodecsQueryOptions(null, true);
        // Keys must differ so TanStack Query caches them separately.
        expect(optsWithContainer.queryKey).not.toEqual(optsNullContainer.queryKey);
    });

    it("uses the same queryKey regardless of ready flag (cache hit when re-enabled)", () => {
        const whenReady = audioCodecsQueryOptions("mkv", true);
        const whenNotReady = audioCodecsQueryOptions("mkv", false);
        expect(whenReady.queryKey).toEqual(whenNotReady.queryKey);
    });
});
