import { describe, it, expect } from "vitest";
import { formStateToPoolIdleTimeout, poolIdleTimeoutToFormState } from "./networkSchema";

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
});
