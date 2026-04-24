// TanStack Store for global UI state: active view, panel collapsed states, selections.

import { Store } from "@tanstack/store";

export type AppView = "analyze" | "queue" | "history" | "settings";

export interface UiState {
    activeView: AppView;
    navPanelCollapsed: boolean;
    bottomDrawerExpanded: boolean;
    selectedFormatId: string | null;
    /**
     * Backend format-selector DSL override (yt-dlp syntax, e.g. `"bv*+ba/best"`).
     *
     * Mutually exclusive with `selectedFormatId`: setters on either field
     * null the other out. Presets like "Best" set this to a DSL string when
     * the extractor exposes both video-only and audio-only renditions so the
     * download side auto-pairs (`bv*+ba/best` falls back to the best muxed on
     * sites that don't offer a split); otherwise they pick a single `format_id`
     * and leave this null.
     *
     * Format-table row highlighting is driven by `selectedFormatId` only —
     * when `selectedSelector` is active no single row is highlighted; the
     * toolbar's preset pill carries the visual state instead.
     */
    selectedSelector: string | null;
    selectedJobId: string | null;
    commandPaletteOpen: boolean;
    analyzeUrl: string | null;
    /** When viewing a single episode from a playlist, stores the playlist URL for back navigation. */
    playlistBackUrl: string | null;
    /**
     * Show video-only / advanced streams in the formats table.
     *
     * HLS renditions that declare a separate AUDIO group (e.g. AV1 variants
     * on XHamster) are marked video-only by the extractor — downloading one
     * by itself produces silent video. We hide that group by default so
     * non-expert users don't pick a silent stream by mistake. Ticking this
     * flag reveals it for users who want to combine video+audio streams
     * manually (or via the backend's format-selection DSL).
     */
    showExpertFormats: boolean;
}

const initialState: UiState = {
    activeView: "analyze",
    navPanelCollapsed: false,
    bottomDrawerExpanded: false,
    selectedFormatId: null,
    selectedSelector: null,
    selectedJobId: null,
    commandPaletteOpen: false,
    analyzeUrl: null,
    playlistBackUrl: null,
    showExpertFormats: false,
};

export const uiStore = new Store<UiState>(initialState);

export function setView(view: AppView): void {
    uiStore.setState((prev) => ({ ...prev, activeView: view }));
}

export function toggleNavPanel(): void {
    uiStore.setState((prev) => ({ ...prev, navPanelCollapsed: !prev.navPanelCollapsed }));
}

export function toggleBottomDrawer(): void {
    uiStore.setState((prev) => ({
        ...prev,
        bottomDrawerExpanded: !prev.bottomDrawerExpanded,
    }));
}

export function setSelectedFormat(id: string | null): void {
    // Mutually exclusive with selectedSelector — picking a single format_id
    // clears any active DSL override so the two states never drift.
    uiStore.setState((prev) => ({
        ...prev,
        selectedFormatId: id,
        selectedSelector: id !== null ? null : prev.selectedSelector,
    }));
}

/**
 * Set a format-selector DSL override (e.g. `"bv*+ba/best"`). Passing a
 * non-null value clears `selectedFormatId`; passing null only clears the
 * selector and leaves any previously-selected format_id untouched.
 */
export function setSelectedSelector(selector: string | null): void {
    uiStore.setState((prev) => ({
        ...prev,
        selectedSelector: selector,
        selectedFormatId: selector !== null ? null : prev.selectedFormatId,
    }));
}

export function setSelectedJob(id: string | null): void {
    uiStore.setState((prev) => ({ ...prev, selectedJobId: id }));
}

export function setCommandPaletteOpen(open: boolean): void {
    uiStore.setState((prev) => ({ ...prev, commandPaletteOpen: open }));
}

export function setActiveView(view: AppView): void {
    uiStore.setState((prev) => ({ ...prev, activeView: view }));
}

export function setAnalyzeUrl(url: string | null): void {
    uiStore.setState((prev) => ({
        ...prev,
        analyzeUrl: url,
        // Clear playlist back URL when navigating to a non-playlist context
        playlistBackUrl: url === null ? null : prev.playlistBackUrl,
    }));
}

export function setShowExpertFormats(value: boolean): void {
    uiStore.setState((prev) => ({ ...prev, showExpertFormats: value }));
}
