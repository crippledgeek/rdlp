// Virtual focus keyboard navigation for the search results table.
// Manages focusIndex; selection is delegated to the TanStack Table instance.

import { useEffect } from "react";
import type { Table } from "@tanstack/react-table";
import type { SearchResultPreview } from "../types";

interface UseKeyboardNavigationOptions {
    focusIndex: number;
    setFocusIndex: React.Dispatch<React.SetStateAction<number>>;
    resultCount: number;
    enabled: boolean;
    table: Table<SearchResultPreview> | null;
    onDownload: (index: number) => void;
    onOpenFormatDialog: (index: number) => void;
    onOpenInBrowser: (index: number) => void;
}

/** Hook providing virtual focus for the results table. Selection is owned by TanStack Table. */
export function useKeyboardNavigation({
    focusIndex,
    setFocusIndex,
    resultCount,
    enabled,
    table,
    onDownload,
    onOpenFormatDialog,
    onOpenInBrowser,
}: UseKeyboardNavigationOptions) {
    // Reset when results change
    useEffect(() => {
        setFocusIndex(-1);
    }, [resultCount, setFocusIndex]);

    useEffect(() => {
        if (!enabled || resultCount === 0) return;

        const handler = (e: KeyboardEvent) => {
            const target = e.target as HTMLElement;
            const tag = target.tagName.toLowerCase();
            // Don't capture when typing in inputs/selects
            if (tag === "input" || tag === "textarea" || tag === "select") return;

            switch (e.key) {
                case "ArrowDown": {
                    e.preventDefault();
                    setFocusIndex((prev) =>
                        prev < resultCount - 1 ? prev + 1 : 0,
                    );
                    break;
                }
                case "ArrowUp": {
                    e.preventDefault();
                    setFocusIndex((prev) =>
                        prev > 0 ? prev - 1 : resultCount - 1,
                    );
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
                    setFocusIndex((prev) =>
                        Math.min(prev + 10, resultCount - 1),
                    );
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
        };

        document.addEventListener("keydown", handler);
        return () => document.removeEventListener("keydown", handler);
    }, [
        enabled,
        resultCount,
        focusIndex,
        table,
        onDownload,
        onOpenFormatDialog,
        onOpenInBrowser,
    ]);

}
