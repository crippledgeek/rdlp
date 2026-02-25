// Unit tests for searchParamsStore.ts — state transitions via setSearchParam
// and resetSearchParams.

import { describe, test, expect, beforeEach } from "vitest";
import { searchParamsAtom, setSearchParam, resetSearchParams } from "./searchParamsStore";

// Reset to initial state before each test to prevent cross-test pollution.
beforeEach(() => {
    resetSearchParams();
});

describe("searchParamsAtom — initial state", () => {
    test("starts with empty query", () => {
        expect(searchParamsAtom.state.query).toBe("");
    });

    test("starts with empty site", () => {
        expect(searchParamsAtom.state.site).toBe("");
    });

    test("starts with empty filters array", () => {
        expect(searchParamsAtom.state.filters).toEqual([]);
    });

    test("starts with hasUserFilters false", () => {
        expect(searchParamsAtom.state.hasUserFilters).toBe(false);
    });
});

describe("setSearchParam", () => {
    test("sets query", () => {
        setSearchParam("query", "cats");
        expect(searchParamsAtom.state.query).toBe("cats");
    });

    test("sets site", () => {
        setSearchParam("site", "xhamster");
        expect(searchParamsAtom.state.site).toBe("xhamster");
    });

    test("sets filters array", () => {
        const filters = [{ key: "ordering", value: "newest" }];
        setSearchParam("filters", filters);
        expect(searchParamsAtom.state.filters).toEqual(filters);
    });

    test("sets hasUserFilters", () => {
        setSearchParam("hasUserFilters", true);
        expect(searchParamsAtom.state.hasUserFilters).toBe(true);
    });

    test("preserves other fields when setting one field", () => {
        setSearchParam("query", "dogs");
        setSearchParam("site", "redtube");
        // Both should be set; filters and hasUserFilters unchanged
        expect(searchParamsAtom.state.query).toBe("dogs");
        expect(searchParamsAtom.state.site).toBe("redtube");
        expect(searchParamsAtom.state.filters).toEqual([]);
        expect(searchParamsAtom.state.hasUserFilters).toBe(false);
    });

    test("overwrites previous value for the same key", () => {
        setSearchParam("query", "first");
        setSearchParam("query", "second");
        expect(searchParamsAtom.state.query).toBe("second");
    });
});

describe("resetSearchParams", () => {
    test("resets query to empty string", () => {
        setSearchParam("query", "something");
        resetSearchParams();
        expect(searchParamsAtom.state.query).toBe("");
    });

    test("resets site to empty string", () => {
        setSearchParam("site", "tnaflix");
        resetSearchParams();
        expect(searchParamsAtom.state.site).toBe("");
    });

    test("resets filters to empty array", () => {
        setSearchParam("filters", [{ key: "ordering", value: "newest" }]);
        resetSearchParams();
        expect(searchParamsAtom.state.filters).toEqual([]);
    });

    test("resets hasUserFilters to false", () => {
        setSearchParam("hasUserFilters", true);
        resetSearchParams();
        expect(searchParamsAtom.state.hasUserFilters).toBe(false);
    });
});
