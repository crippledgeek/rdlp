// Unit tests for queryKeys.ts — deterministic key generation.

import { describe, test, expect } from "vitest";
import { queryKeys } from "./queryKeys";
import type { SearchFilter } from "../types";

describe("queryKeys.providers", () => {
    test("returns ['providers']", () => {
        expect(queryKeys.providers()).toEqual(["providers"]);
    });
});

describe("queryKeys.filters", () => {
    test("includes the site name", () => {
        expect(queryKeys.filters("xhamster")).toEqual(["filters", "xhamster"]);
    });
});

describe("queryKeys.search.all", () => {
    test("returns ['search']", () => {
        expect(queryKeys.search.all()).toEqual(["search"]);
    });
});

describe("queryKeys.search.site", () => {
    test("returns ['search', site]", () => {
        expect(queryKeys.search.site("redtube")).toEqual(["search", "redtube"]);
    });
});

describe("queryKeys.search.params", () => {
    test("returns a 4-element tuple", () => {
        const key = queryKeys.search.params("cats", "tnaflix", []);
        expect(key).toHaveLength(4);
        expect(key[0]).toBe("search");
        expect(key[1]).toBe("tnaflix");
        expect(key[2]).toBe("cats");
    });

    test("serializes empty filters as empty string", () => {
        const key = queryKeys.search.params("dogs", "redtube", []);
        expect(key[3]).toBe("");
    });

    test("serializes single filter as key=value", () => {
        const filters: SearchFilter[] = [{ key: "ordering", value: "newest" }];
        const key = queryKeys.search.params("cats", "redtube", filters);
        expect(key[3]).toBe("ordering=newest");
    });

    test("serializes multiple filters sorted alphabetically", () => {
        const filters: SearchFilter[] = [
            { key: "period", value: "weekly" },
            { key: "ordering", value: "newest" },
        ];
        const key = queryKeys.search.params("cats", "redtube", filters);
        // sorted: ordering before period
        expect(key[3]).toBe("ordering=newest&period=weekly");
    });

    test("is deterministic regardless of filter insertion order", () => {
        const filtersA: SearchFilter[] = [
            { key: "ordering", value: "newest" },
            { key: "period", value: "weekly" },
        ];
        const filtersB: SearchFilter[] = [
            { key: "period", value: "weekly" },
            { key: "ordering", value: "newest" },
        ];
        const keyA = queryKeys.search.params("q", "redtube", filtersA);
        const keyB = queryKeys.search.params("q", "redtube", filtersB);
        expect(keyA[3]).toBe(keyB[3]);
    });

    test("different filter values produce different keys", () => {
        const a = queryKeys.search.params("q", "redtube", [{ key: "ordering", value: "newest" }]);
        const b = queryKeys.search.params("q", "redtube", [{ key: "ordering", value: "rating" }]);
        expect(a[3]).not.toBe(b[3]);
    });
});

describe("queryKeys.downloads.list", () => {
    test("returns ['downloads']", () => {
        expect(queryKeys.downloads.list()).toEqual(["downloads"]);
    });
});

describe("queryKeys.formats", () => {
    test("includes the url", () => {
        expect(queryKeys.formats("https://example.com/video")).toEqual([
            "formats",
            "https://example.com/video",
        ]);
    });
});

describe("queryKeys.settings", () => {
    test("returns ['settings']", () => {
        expect(queryKeys.settings()).toEqual(["settings"]);
    });
});
