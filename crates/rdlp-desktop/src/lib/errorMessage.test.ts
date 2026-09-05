// @vitest-environment node

import { describe, test, expect } from "vitest";
import { errorMessage } from "@/lib/errorMessage";

describe("errorMessage", () => {
    test("uses an Error's message", () => {
        expect(errorMessage(new Error("boom"), "fallback")).toBe("boom");
    });

    // The shape a Tauri invoke rejection actually takes: a plain object, not
    // an Error. `String(err)` on this yields "[object Object]", which is the
    // defect this helper exists to prevent.
    test("uses the message field of a plain object", () => {
        expect(errorMessage({ message: "File not found: /x" }, "fallback")).toBe(
            "File not found: /x",
        );
    });

    test("uses a non-empty string as-is", () => {
        expect(errorMessage("plain failure", "fallback")).toBe("plain failure");
    });

    test.each([[null], [undefined], [42], [{}], [""]])(
        "falls back when nothing readable is present (%p)",
        (value) => {
            expect(errorMessage(value, "fallback")).toBe("fallback");
        },
    );
});
