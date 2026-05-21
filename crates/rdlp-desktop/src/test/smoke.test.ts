// Smoke test — verifies Vitest + Testing Library infrastructure is wired up.

import { describe, it, expect, afterEach } from "vitest";
import { clearInvokeHandlers, setInvokeHandler, mockInvoke } from "./tauri-mock";
import { clearEventListeners, mockListen, mockEmit } from "./tauri-mock";

describe("Vitest infrastructure", () => {
    afterEach(() => {
        clearInvokeHandlers();
        clearEventListeners();
    });

    it("runs a basic assertion", () => {
        expect(1 + 1).toBe(2);
    });

    it("jest-dom matchers are available", () => {
        const el = document.createElement("div");
        el.textContent = "hello";
        document.body.appendChild(el);
        expect(el).toBeInTheDocument();
        document.body.removeChild(el);
    });

    it("mockInvoke returns undefined for unregistered commands", async () => {
        const result = await mockInvoke("unknown_command");
        expect(result).toBeUndefined();
    });

    it("mockInvoke routes to registered handler", async () => {
        setInvokeHandler("queue", () => []);
        const result = await mockInvoke<unknown[]>("queue");
        expect(result).toEqual([]);
    });

    it("mockListen + mockEmit delivers payloads to listeners", async () => {
        const received: unknown[] = [];
        await mockListen("download-progress", (event) => {
            received.push(event.payload);
        });
        await mockEmit("download-progress", { percent: 42 });
        expect(received).toHaveLength(1);
        expect(received[0]).toEqual({ percent: 42 });
    });
});
