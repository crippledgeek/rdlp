// Virtual focus keyboard navigation for the search results table.
// Owns focusIndex internally; selection is delegated to the TanStack Table
// instance passed in by the caller.

import { useCallback, useEffect, useState } from "react";
import type { Table } from "@tanstack/react-table";
import type { SearchResultPreview } from "../types";

interface UseKeyboardNavigationOptions {
    /** Number of rows currently rendered in the table. */
    resultCount: number;
    /** Whether keyboard handling is active. */
    enabled: boolean;
    /** Underlying TanStack Table instance for selection toggles. */
    table: Table<SearchResultPreview> | null;
    /** Triggered when the user presses D on a focused row. */
    onDownload: (index: number) => void;
    /** Triggered on Enter — open the format chooser for the focused row. */
    onOpenFormatDialog: (index: number) => void;
    /** Triggered on O — open the source URL in the system browser. */
    onOpenInBrowser: (index: number) => void;
}

interface UseKeyboardNavigationResult {
    /** Currently focused row index, or -1 if no row is focused. */
    focusIndex: number;
}

/**
 * Hook providing virtual focus for the results table.
 *
 * Returns the current `focusIndex`. When `resultCount` changes (e.g. a new
 * search loads), the focus is cleared automatically via React's supported
 * "adjust state during render" pattern — no Effect required.
 */
export function useKeyboardNavigation({
    resultCount,
    enabled,
    table,
    onDownload,
    onOpenFormatDialog,
    onOpenInBrowser,
}: UseKeyboardNavigationOptions): UseKeyboardNavigationResult {
    const [focusIndex, setFocusIndex] = useState(-1);

    // Reset focus when the data set changes. React's documented pattern for
    // resetting state in response to a prop change without useEffect:
    // compare against the previous prop value tracked in its own useState,
    // and call both setters during render. React short-circuits the render
    // and re-runs with the new state — no extra commit / repaint.
    // See: https://react.dev/reference/react/useState#storing-information-from-previous-renders
    const [lastSeenCount, setLastSeenCount] = useState(resultCount);
    if (lastSeenCount !== resultCount) {
        setLastSeenCount(resultCount);
        setFocusIndex(-1);
    }

    // Keyboard handler lives on `document` — a genuine external-system
    // subscription, so useEffect is the correct primitive per the
    // dont-use-use-effect skill §7.
    const handleKey = useCallback(
        (e: KeyboardEvent) => {
            if (!enabled || resultCount === 0) return;
            const target = e.target as HTMLElement;
            const tag = target.tagName.toLowerCase();
            // Don't capture when typing in inputs/selects
            if (tag === "input" || tag === "textarea" || tag === "select") return;

            switch (e.key) {
                case "ArrowDown": {
                    e.preventDefault();
                    setFocusIndex((prev) => (prev < resultCount - 1 ? prev + 1 : 0));
                    break;
                }
                case "ArrowUp": {
                    e.preventDefault();
                    setFocusIndex((prev) => (prev > 0 ? prev - 1 : resultCount - 1));
                    break;
                }
                case "Home": {
                    e.preventDefault();
                    setFocusIndex(0);
                    break;
                }
                case "End": {
                    e.preventDefault();
                    setFocusIndex(resultCount - 1);
                    break;
                }
                case "PageDown": {
                    e.preventDefault();
                    setFocusIndex((prev) => Math.min(prev + 10, resultCount - 1));
                    break;
                }
                case "PageUp": {
                    e.preventDefault();
                    setFocusIndex((prev) => Math.max(prev - 10, 0));
                    break;
                }
                case " ": {
                    e.preventDefault();
                    if (focusIndex >= 0 && table) {
                        const row = table.getRowModel().rows[focusIndex];
                        row?.toggleSelected();
                    }
                    break;
                }
                case "Enter": {
                    e.preventDefault();
                    if (focusIndex >= 0) onOpenFormatDialog(focusIndex);
                    break;
                }
                case "d":
                case "D": {
                    if (focusIndex >= 0) {
                        e.preventDefault();
                        onDownload(focusIndex);
                    }
                    break;
                }
                case "o":
                case "O": {
                    if (focusIndex >= 0) {
                        e.preventDefault();
                        onOpenInBrowser(focusIndex);
                    }
                    break;
                }
                case "Escape": {
                    if (table && table.getSelectedRowModel().rows.length > 0) {
                        table.toggleAllRowsSelected(false);
                    } else {
                        setFocusIndex(-1);
                    }
                    break;
                }
                case "a": {
                    if (e.ctrlKey || e.metaKey) {
                        e.preventDefault();
                        table?.toggleAllRowsSelected(true);
                    }
                    break;
                }
            }
        },
        [enabled, resultCount, focusIndex, table, onDownload, onOpenFormatDialog, onOpenInBrowser],
    );

    useEffect(() => {
        if (!enabled || resultCount === 0) return;
        document.addEventListener("keydown", handleKey);
        return () => document.removeEventListener("keydown", handleKey);
    }, [enabled, resultCount, handleKey]);

    return { focusIndex };
}
