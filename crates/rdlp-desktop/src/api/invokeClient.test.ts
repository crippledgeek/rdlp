// @vitest-environment node

import { describe, test, expect } from "vitest";
import { extractErrorMessage } from "@/api/invokeClient";

// `extractErrorMessage` is the single unwrap for a value caught from an
// `invoke` rejection. It had no tests; these arrived with a would-be second
// helper (#693) and were retargeted here when that duplicate was dropped.
describe("extractErrorMessage", () => {
    test("uses an Error's message", () => {
        expect(extractErrorMessage(new Error("boom"))).toBe("boom");
    });

    // The shape a Tauri invoke rejection actually takes: a plain object, not
    // an Error. `String(err)` on this yields "[object Object]".
    test("uses the message field of a plain object", () => {
        expect(extractErrorMessage({ message: "File not found: /x" })).toBe(
            "File not found: /x",
        );
    });

    // AppError is `#[serde(tag = "kind", content = "data")]`, so a rejection
    // arriving straight from `invoke` (not through invokeTyped's flattening)
    // nests the message one level down. The helper this replaced had no branch
    // for it and would have returned "[object Object]".
    test("reads through the adjacently-tagged AppError shape", () => {
        expect(
            extractErrorMessage({
                kind: "SearchFailed",
                data: { message: "upstream refused", retryable: true },
            }),
        ).toBe("upstream refused");
    });

    test("uses a string as-is", () => {
        expect(extractErrorMessage("plain failure")).toBe("plain failure");
    });

    // Readable JSON beats "[object Object]" for a shape nothing recognises.
    test("falls back to JSON for an unrecognised object", () => {
        expect(extractErrorMessage({ code: 7 })).toBe('{"code":7}');
    });

    test.each([[null], [undefined], [42]])(
        "stringifies a non-object value (%p)",
        (value) => {
            expect(extractErrorMessage(value)).toBe(String(value));
        },
    );
});
