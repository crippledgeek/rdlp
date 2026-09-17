// The QueryClient must never re-invoke a failed IPC command (#700).
//
// Every Tauri command is the engine's FINAL verdict: rdlp-core already retries
// transient network failures internally (`Config::retries`, `is_retryable`), so
// a rejected `invoke` means the engine gave up. A frontend retry re-runs the
// whole extraction/search against the target site — measured as two `formats`
// command entries ~600 ms apart on a failing Analyze, doubling rate-limit and
// ban exposure for nothing.

import { describe, expect, it } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { queryClient } from "./queryClient";

describe("queryClient retry policy", () => {
    it("invokes a failing queryFn exactly once", async () => {
        // A private client built from the app client's REAL default options:
        // the policy under test is exercised, but nothing is written into the
        // shared singleton's cache for other tests to trip over.
        const client = new QueryClient({ defaultOptions: queryClient.getDefaultOptions() });
        let calls = 0;
        const boom = new Error("extraction failed");
        await expect(
            client.fetchQuery({
                queryKey: ["retry-policy-probe"],
                queryFn: () => {
                    calls += 1;
                    return Promise.reject(boom);
                },
            }),
        ).rejects.toBe(boom);
        expect(calls).toBe(1);
    });
});
