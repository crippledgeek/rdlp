import { describe, it, expect } from "vitest";
import { cancelledJobPatch } from "./jobStatus";

describe("cancelledJobPatch", () => {
    it("sets cancelled status and clears all transient progress fields", () => {
        expect(cancelledJobPatch()).toEqual({
            status: "cancelled",
            statusMessage: null,
            currentUnit: null,
            speed: null,
            eta: null,
            progress: null,
        });
    });
});
