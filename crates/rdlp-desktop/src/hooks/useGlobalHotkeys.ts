// Global keyboard shortcuts for the pane-based workspace.
// Uses @tanstack/react-hotkeys `useHotkey` hook.
// RawHotkey objects are used to avoid type-safe string literal restrictions.

import { useHotkey } from "@tanstack/react-hotkeys";
import type { RawHotkey } from "@tanstack/react-hotkeys";
import {
    setView,
    toggleNavPanel,
    toggleBottomDrawer,
    setCommandPaletteOpen,
} from "@/stores/uiStore";

const hotkey = (ctrl: boolean, shift: boolean, key: string): RawHotkey => ({
    key,
    ...(ctrl && { ctrl: true }),
    ...(shift && { shift: true }),
});

export function useGlobalHotkeys() {
    // Ctrl+1: Analyze view
    useHotkey(hotkey(true, false, "1"), () => setView("analyze"), { preventDefault: true });

    // Ctrl+2: Queue view
    useHotkey(hotkey(true, false, "2"), () => setView("queue"), { preventDefault: true });

    // Ctrl+3: History view
    useHotkey(hotkey(true, false, "3"), () => setView("history"), { preventDefault: true });

    // Ctrl+4: Settings view
    useHotkey(hotkey(true, false, "4"), () => setView("settings"), { preventDefault: true });

    // Ctrl+B: Toggle nav panel
    useHotkey(hotkey(true, false, "b"), () => toggleNavPanel(), { preventDefault: true });

    // Ctrl+J: Toggle bottom drawer
    useHotkey(hotkey(true, false, "j"), () => toggleBottomDrawer(), { preventDefault: true });

    // Ctrl+Shift+P: Open command palette
    useHotkey(hotkey(true, true, "p"), () => setCommandPaletteOpen(true), { preventDefault: true });

    // Ctrl+K: Focus command bar
    useHotkey(hotkey(true, false, "k"), () => {
        window.dispatchEvent(new CustomEvent("rdlp-focus-search"));
    }, { preventDefault: true });

    // Ctrl+Enter: Download selected format
    useHotkey(hotkey(true, false, "Enter"), () => {
        window.dispatchEvent(new CustomEvent("rdlp-download-selected"));
    }, { preventDefault: true });
}
