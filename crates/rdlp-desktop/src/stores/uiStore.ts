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
}

const initialState: UiState = {
    activeView: "analyze",
    navPanelCollapsed: false,
    bottomDrawerExpanded: false,
    selectedFormatId: null,
    selectedJobId: null,
    commandPaletteOpen: false,
    analyzeUrl: null,
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

export function setAnalyzeUrl(url: string | null): void {
    uiStore.setState((prev) => ({ ...prev, analyzeUrl: url }));
}
