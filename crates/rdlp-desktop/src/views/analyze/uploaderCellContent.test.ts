import { describe, it, expect } from "vitest";
import { uploaderCellState } from "./uploaderCellContent";

describe("uploaderCellState", () => {
    it("returns loaded with the uploader text when present", () => {
        const s = uploaderCellState({
            uploader: "Pearl Sinclair",
            isEnriching: false,
            enrichmentResolved: true,
        });
        expect(s).toEqual({ kind: "loaded", content: "Pearl Sinclair" });
    });

    it("returns loading while enrichment is in flight and no value yet", () => {
        const s = uploaderCellState({
            uploader: null,
            isEnriching: true,
            enrichmentResolved: false,
        });
        expect(s.kind).toBe("loading");
    });

    /// REGRESSION: prior to commit 5542966 the post-enrichment empty state
    /// rendered an em-dash directly in `SearchResultRow`. The user could
    /// not visually distinguish "we tried and the page has nothing" from
    /// generic missing-data placeholders elsewhere in the table. The
    /// helper now reports `unavailable` for this state so the UI layer
    /// can render a clear "Unavailable" label.
    it("returns unavailable when enrichment resolved with no uploader", () => {
        const s = uploaderCellState({
            uploader: null,
            isEnriching: false,
            enrichmentResolved: true,
        });
        expect(s.kind).toBe("unavailable");
    });

    it("returns fallthrough before enrichment has been attempted", () => {
        const s = uploaderCellState({
            uploader: null,
            isEnriching: false,
            enrichmentResolved: false,
        });
        expect(s.kind).toBe("fallthrough");
    });

    it("treats empty string as missing (some sites surface '' instead of null)", () => {
        const s = uploaderCellState({
            uploader: "",
            isEnriching: false,
            enrichmentResolved: true,
        });
        expect(s.kind).toBe("unavailable");
    });

    it("prefers a loaded value over loading even if a refetch is in flight", () => {
        // Realistic scenario: cached enrichment hit, react-query
        // background-revalidates while still serving the cached value.
        const s = uploaderCellState({
            uploader: "Cached Studio",
            isEnriching: true,
            enrichmentResolved: true,
        });
        expect(s).toEqual({ kind: "loaded", content: "Cached Studio" });
    });
});
