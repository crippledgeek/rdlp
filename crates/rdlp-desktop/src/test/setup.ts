// Global test setup: jest-dom matchers + Tauri IPC mocks + jsdom polyfills.

import "@testing-library/jest-dom";
import { mockInvoke, mockListen, mockEmit } from "./tauri-mock";

// Polyfill pointer-capture methods missing in jsdom.
// Radix UI Select uses hasPointerCapture internally, which throws in jsdom.
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

// Mock @tauri-apps/api/core so tests never hit the real Tauri IPC bridge.
vi.mock("@tauri-apps/api/core", () => ({
    invoke: mockInvoke,
}));

// Mock @tauri-apps/api/event so event listeners are captured in tests.
vi.mock("@tauri-apps/api/event", () => ({
    listen: mockListen,
    emit: mockEmit,
}));
