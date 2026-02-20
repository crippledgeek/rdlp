// Virtual focus + selection keyboard navigation for the results list.
// Manages focusIndex and selectedIndices without using DOM focus.

import { useCallback, useEffect, useState } from "react";

export interface KeyboardNavState {
    focusIndex: number;
    selectedIndices: Set<number>;
}

interface UseKeyboardNavigationOptions {
    resultCount: number;
    enabled: boolean;
    onDownload: (index: number) => void;
    onOpenFormatDialog: (index: number) => void;
    onOpenInBrowser: (index: number) => void;
}

/** Hook providing virtual focus + selection for the results list. */
export function useKeyboardNavigation({
    resultCount,
    enabled,
    onDownload,
    onOpenFormatDialog,
    onOpenInBrowser,
}: UseKeyboardNavigationOptions) {
    const [focusIndex, setFocusIndex] = useState(-1);
    const [selectedIndices, setSelectedIndices] = useState<Set<number>>(
        () => new Set(),
    );

    // Reset when results change
    useEffect(() => {
        setFocusIndex(-1);
        setSelectedIndices(new Set());
    }, [resultCount]);

    const toggleSelect = useCallback((index: number) => {
        setSelectedIndices((prev) => {
            const next = new Set(prev);
            if (next.has(index)) {
                next.delete(index);
            } else {
                next.add(index);
            }
            return next;
        });
    }, []);

    const selectAll = useCallback(() => {
        const all = new Set<number>();
        for (let i = 0; i < resultCount; i++) all.add(i);
        setSelectedIndices(all);
    }, [resultCount]);

    const clearSelection = useCallback(() => {
        setSelectedIndices(new Set());
    }, []);

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
                    if (focusIndex >= 0) toggleSelect(focusIndex);
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
                    if (selectedIndices.size > 0) {
                        clearSelection();
                    } else {
                        setFocusIndex(-1);
                    }
                    break;
                }
                case "a": {
                    if (e.ctrlKey || e.metaKey) {
                        e.preventDefault();
                        selectAll();
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
        selectedIndices.size,
        onDownload,
        onOpenFormatDialog,
        onOpenInBrowser,
        toggleSelect,
        selectAll,
        clearSelection,
    ]);

    return {
        focusIndex,
        selectedIndices,
        toggleSelect,
        selectAll,
        clearSelection,
        setFocusIndex,
    };
}
