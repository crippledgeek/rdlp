import { describe, it, expect } from "vitest";
import {
    socketTimeoutSchema,
    readTimeoutSchema,
    poolIdleTimeoutSchema,
    formStateToPoolIdleTimeout,
    poolIdleTimeoutToFormState,
} from "./networkSchema";

describe("socketTimeoutSchema", () => {
    it("accepts an empty string as null (use default)", () => {
        expect(socketTimeoutSchema.parse("")).toBeNull();
    });
    it("accepts integer 1..=300", () => {
        expect(socketTimeoutSchema.parse("30")).toBe(30);
        expect(socketTimeoutSchema.parse("300")).toBe(300);
    });
    it("rejects 0 (validate.rs requires >=1)", () => {
        expect(() => socketTimeoutSchema.parse("0")).toThrow();
    });
    it("rejects values above 300", () => {
        expect(() => socketTimeoutSchema.parse("301")).toThrow();
    });
    it("rejects negative numbers", () => {
        expect(() => socketTimeoutSchema.parse("-1")).toThrow();
    });
    it("rejects non-integers", () => {
        expect(() => socketTimeoutSchema.parse("3.5")).toThrow();
    });
    it("rejects garbage", () => {
        expect(() => socketTimeoutSchema.parse("abc")).toThrow();
    });
});

describe("readTimeoutSchema", () => {
    it("accepts integer up to 600", () => {
        expect(readTimeoutSchema.parse("600")).toBe(600);
    });
    it("rejects above 600", () => {
        expect(() => readTimeoutSchema.parse("601")).toThrow();
    });
    it("rejects 0", () => {
        expect(() => readTimeoutSchema.parse("0")).toThrow();
    });
});

describe("poolIdleTimeoutSchema (numeric input only — does not see the checkbox)", () => {
    it("accepts integer 1..=3600", () => {
        expect(poolIdleTimeoutSchema.parse("90")).toBe(90);
        expect(poolIdleTimeoutSchema.parse("3600")).toBe(3600);
    });
    it("rejects 0 with the use-checkbox hint (the checkbox produces 0, not the input)", () => {
        expect(() => poolIdleTimeoutSchema.parse("0")).toThrow(/checkbox/i);
    });
    it("rejects above 3600", () => {
        expect(() => poolIdleTimeoutSchema.parse("3601")).toThrow();
    });
    it("accepts empty string as null", () => {
        expect(poolIdleTimeoutSchema.parse("")).toBeNull();
    });
});

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
