import { describe, it, expect } from "vitest";
import { POOL_IDLE_DISABLED, formStateToPoolIdleTimeout, poolIdleNumericMin, poolIdleTimeoutToFormState } from "./networkSchema";

describe("pool-idle form mapping", () => {
    it("checkbox off → 0 (sentinel)", () => {
        expect(formStateToPoolIdleTimeout({ evictIdle: false, secondsInput: "90" })).toBe(0);
        expect(formStateToPoolIdleTimeout({ evictIdle: false, secondsInput: "" })).toBe(0);
    });
    it("checkbox on + numeric input → that integer", () => {
        expect(formStateToPoolIdleTimeout({ evictIdle: true, secondsInput: "120" })).toBe(120);
    });
    it("checkbox on + empty input → null (use default)", () => {
        expect(formStateToPoolIdleTimeout({ evictIdle: true, secondsInput: "" })).toBeNull();
    });
    it("hydrate: 0 → checkbox off, numeric stays empty", () => {
        expect(poolIdleTimeoutToFormState(0)).toEqual({ evictIdle: false, secondsInput: "" });
    });
    it("hydrate: positive → checkbox on, numeric populated", () => {
        expect(poolIdleTimeoutToFormState(90)).toEqual({ evictIdle: true, secondsInput: "90" });
    });
    it("hydrate: null → checkbox on, numeric stays empty", () => {
        expect(poolIdleTimeoutToFormState(null)).toEqual({ evictIdle: true, secondsInput: "" });
    });

    // The owner's range starts AT the sentinel; the numeric control starts one
    // past it, and otherwise follows the owner's min.
    it("poolIdleNumericMin excludes the 0 sentinel and follows a higher owner min", () => {
        expect(POOL_IDLE_DISABLED).toBe(0);
        expect(poolIdleNumericMin({ min: POOL_IDLE_DISABLED, max: 3600 })).toBe(1);
        expect(poolIdleNumericMin({ min: 5, max: 3600 })).toBe(5);
    });
});
