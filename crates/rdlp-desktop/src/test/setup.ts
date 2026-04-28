// Global test setup: jest-dom matchers + Tauri IPC mocks + DOM polyfills.

import "@testing-library/jest-dom";
import { mockInvoke, mockListen, mockEmit } from "./tauri-mock";

// Polyfill pointer-capture methods missing in some DOM environments.
// Radix UI Select uses hasPointerCapture internally.
// Guard against node environment where Element is undefined.
if (typeof Element !== "undefined") {
    if (typeof Element.prototype.hasPointerCapture !== "function") {
        Element.prototype.hasPointerCapture = () => false;
    }
    if (typeof Element.prototype.setPointerCapture !== "function") {
        Element.prototype.setPointerCapture = () => {};
    }
    if (typeof Element.prototype.releasePointerCapture !== "function") {
        Element.prototype.releasePointerCapture = () => {};
    }
    // Polyfill scrollIntoView (used by Radix Select for option scrolling).
    if (typeof Element.prototype.scrollIntoView !== "function") {
        Element.prototype.scrollIntoView = () => {};
    }
}

// Mock @tauri-apps/api/core so tests never hit the real Tauri IPC bridge.
vi.mock("@tauri-apps/api/core", () => ({
    invoke: mockInvoke,
}));

// Mock @tauri-apps/api/event so event listeners are captured in tests.
vi.mock("@tauri-apps/api/event", () => ({
    listen: mockListen,
    emit: mockEmit,
}));

// Replace the React Aria (Jolly UI) Select with a lightweight test stub.
// The real component uses portals, popover state, and full ARIA wiring
// that costs ~40-80ms per render. The mock preserves the ARIA roles
// (combobox, option, radio) that tests query for.
vi.mock("@/components/ui/select", async () => import("./select-mock"));
