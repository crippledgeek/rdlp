// @vitest-environment node
// Unit tests for searchUtils.ts — formatDuration, formatCompactDate, formatViews.

import { describe, test, expect } from "vitest";
import { formatDuration, formatViews, formatCompactDate } from "./searchUtils";

// ---------------------------------------------------------------------------
// formatDuration
// ---------------------------------------------------------------------------

describe("formatDuration", () => {
    test("returns em-dash for null", () => {
        expect(formatDuration(null)).toBe("\u2014");
    });

    test("formats 0 seconds as 0:00", () => {
        expect(formatDuration(0)).toBe("0:00");
    });

    test("pads seconds below 10", () => {
        expect(formatDuration(65)).toBe("1:05");
    });

    test("formats exactly 1 minute", () => {
        expect(formatDuration(60)).toBe("1:00");
    });

    test("formats multi-digit minutes correctly", () => {
        expect(formatDuration(3661)).toBe("61:01");
    });

    test("truncates fractional seconds", () => {
        expect(formatDuration(90.9)).toBe("1:30");
    });
});

// ---------------------------------------------------------------------------
// formatViews
// ---------------------------------------------------------------------------

describe("formatViews", () => {
    test("returns empty string for null", () => {
        expect(formatViews(null)).toBe("");
    });

    test("formats sub-thousand count as plain number", () => {
        expect(formatViews(999)).toBe("999 views");
    });

    test("formats thousands with K suffix", () => {
        expect(formatViews(1500)).toBe("1.5K views");
    });

    test("formats millions with M suffix", () => {
        expect(formatViews(2_500_000)).toBe("2.5M views");
    });

    test("formats exactly 1000 as K", () => {
        expect(formatViews(1000)).toBe("1.0K views");
    });

    test("formats exactly 1 million as M", () => {
        expect(formatViews(1_000_000)).toBe("1.0M views");
    });

    test("formats zero", () => {
        expect(formatViews(0)).toBe("0 views");
    });
});

// ---------------------------------------------------------------------------
// formatCompactDate
// ---------------------------------------------------------------------------

describe("formatCompactDate", () => {
    test("returns empty string for null", () => {
        expect(formatCompactDate(null)).toBe("");
    });

    test("returns empty string for unrecognised timestamp", () => {
        expect(formatCompactDate("not-a-date")).toBe("");
    });

    test("returns a non-empty string for a valid recent Unix timestamp", () => {
        // A timestamp roughly 1 year in the past
        const oneYearAgo = Math.floor(Date.now() / 1000) - 365 * 24 * 60 * 60;
        const result = formatCompactDate(String(oneYearAgo));
        expect(result).not.toBe("");
        // Should contain digits
        expect(result).toMatch(/\d/);
    });

    test("returns a compact suffix (no full words like 'days' or 'hours')", () => {
        const threeDaysAgo = Math.floor(Date.now() / 1000) - 3 * 24 * 60 * 60;
        const result = formatCompactDate(String(threeDaysAgo));
        // Should not include the word 'days'
        expect(result).not.toContain("days");
        expect(result).not.toContain("day");
    });

    test("returns empty string for empty string input", () => {
        expect(formatCompactDate("")).toBe("");
    });
});
