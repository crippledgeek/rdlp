// @vitest-environment node
// Unit tests for lib/utils.ts — cn() and parseUploadTimestamp().

import { describe, test, expect } from "vitest";
import { cn, displayTitle, parseUploadTimestamp, TITLE_PLACEHOLDER } from "./utils";

// ---------------------------------------------------------------------------
// cn()
// ---------------------------------------------------------------------------

describe("cn", () => {
    test("joins class strings", () => {
        expect(cn("foo", "bar")).toBe("foo bar");
    });

    test("handles conditional falsy values", () => {
        expect(cn("foo", false && "bar", undefined, null, "baz")).toBe("foo baz");
    });

    test("merges conflicting tailwind classes (last wins)", () => {
        // twMerge behaviour: later class overrides earlier same-utility class
        expect(cn("p-2", "p-4")).toBe("p-4");
    });

    test("handles an empty call", () => {
        expect(cn()).toBe("");
    });

    test("handles object syntax", () => {
        expect(cn({ "text-red-500": true, "text-blue-500": false })).toBe("text-red-500");
    });

    test("handles array syntax", () => {
        expect(cn(["foo", "bar"])).toBe("foo bar");
    });
});

// ---------------------------------------------------------------------------
// parseUploadTimestamp()
// ---------------------------------------------------------------------------

describe("parseUploadTimestamp", () => {
    test("returns null for empty string", () => {
        expect(parseUploadTimestamp("")).toBeNull();
    });

    test("returns null for whitespace-only string", () => {
        expect(parseUploadTimestamp("   ")).toBeNull();
    });

    test("parses Unix timestamp (10 digits)", () => {
        const d = parseUploadTimestamp("1700000000");
        expect(d).not.toBeNull();
        expect(d!.getTime()).toBe(1700000000 * 1000);
    });

    test("parses Unix timestamp (9 digits)", () => {
        const d = parseUploadTimestamp("946684800");
        expect(d).not.toBeNull();
        expect(d!.getTime()).toBe(946684800 * 1000);
    });

    test("parses YYYYMMDD compact date", () => {
        const d = parseUploadTimestamp("20230101");
        expect(d).not.toBeNull();
        expect(d!.getFullYear()).toBe(2023);
        expect(d!.getMonth()).toBe(0); // January
        expect(d!.getDate()).toBe(1);
    });

    test("parses YYYY-MM-DD ISO date", () => {
        const d = parseUploadTimestamp("2024-06-15");
        expect(d).not.toBeNull();
        expect(d!.getFullYear()).toBe(2024);
        expect(d!.getMonth()).toBe(5); // June
        expect(d!.getDate()).toBe(15);
    });

    test("parses YYYY-MM-DD HH:MM:SS datetime", () => {
        const d = parseUploadTimestamp("2024-06-15 10:30:00");
        expect(d).not.toBeNull();
        expect(d!.getFullYear()).toBe(2024);
    });

    test("returns null for unrecognized formats", () => {
        expect(parseUploadTimestamp("June 15, 2024")).toBeNull();
        expect(parseUploadTimestamp("not-a-date")).toBeNull();
    });

    test("trims surrounding whitespace before parsing", () => {
        const d = parseUploadTimestamp("  20230101  ");
        expect(d).not.toBeNull();
        expect(d!.getFullYear()).toBe(2023);
    });
});

// ---------------------------------------------------------------------------
// displayTitle()
// ---------------------------------------------------------------------------

describe("displayTitle", () => {
    test("returns trimmed title for non-empty input", () => {
        expect(displayTitle("My Video")).toBe("My Video");
        expect(displayTitle("  Padded  ")).toBe("Padded");
    });

    test("returns placeholder for null", () => {
        expect(displayTitle(null)).toBe(TITLE_PLACEHOLDER);
    });

    test("returns placeholder for undefined", () => {
        expect(displayTitle(undefined)).toBe(TITLE_PLACEHOLDER);
    });

    test("returns placeholder for empty string", () => {
        // godresource regression: API returns title:null, plugin's
        // _real_extract sets `'title': ''` — must surface as "Unknown".
        expect(displayTitle("")).toBe(TITLE_PLACEHOLDER);
    });

    test("returns placeholder for whitespace-only string", () => {
        expect(displayTitle("   ")).toBe(TITLE_PLACEHOLDER);
        expect(displayTitle("\t\n")).toBe(TITLE_PLACEHOLDER);
    });

    test("placeholder text is 'Unknown'", () => {
        // Locked in so a future rename of the constant doesn't silently
        // change what users see; rename here too if you change the
        // shared placeholder.
        expect(TITLE_PLACEHOLDER).toBe("Unknown");
    });
});
