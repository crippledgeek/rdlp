import { describe, it, expect } from "vitest";
import { bytesToMibDisplay, mibDisplayToBytes, BYTES_PER_MIB } from "./byteUnits";

describe("byteUnits", () => {
    it("converts whole MiB to bytes exactly", () => {
        expect(mibDisplayToBytes(1)).toBe(1_048_576);
        expect(mibDisplayToBytes(2)).toBe(2_097_152);
        expect(mibDisplayToBytes(1024)).toBe(1_073_741_824);
    });

    it("converts bytes to whole MiB for display", () => {
        expect(bytesToMibDisplay(2_097_152)).toBe(2);
        expect(bytesToMibDisplay(10_485_760)).toBe(10);
    });

    it("round-trips the shipped defaults exactly", () => {
        // Config defaults are exact MiB multiples, so display→store is lossless.
        expect(mibDisplayToBytes(bytesToMibDisplay(2 * BYTES_PER_MIB))).toBe(2 * BYTES_PER_MIB);
        expect(mibDisplayToBytes(bytesToMibDisplay(10 * BYTES_PER_MIB))).toBe(10 * BYTES_PER_MIB);
    });

    // Documents the lossy direction: a hand-edited non-whole-MiB byte value cannot
    /// survive a display round-trip. The UI's defence is to never write back a field
    /// the user did not edit — this test pins WHY that rule exists.
    it("is lossy for non-whole-MiB byte values (display rounds)", () => {
        expect(bytesToMibDisplay(10_500_000)).toBe(10);
        expect(mibDisplayToBytes(bytesToMibDisplay(10_500_000))).not.toBe(10_500_000);
    });

    it("rounds sub-MiB values to zero in display", () => {
        expect(bytesToMibDisplay(500_000)).toBe(0);
    });
});
