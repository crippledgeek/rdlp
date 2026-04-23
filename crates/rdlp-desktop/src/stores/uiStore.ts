// TanStack Store for global UI state: active view, panel collapsed states, selections.

import { Store } from "@tanstack/store";

export type AppView = "analyze" | "queue" | "history" | "settings";

export interface UiState {
    activeView: AppView;
    navPanelCollapsed: boolean;
    bottomDrawerExpanded: boolean;
    selectedFormatId: string | null;
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
    uiStore.setState((prev) => ({ ...prev, selectedFormatId: id }));
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
