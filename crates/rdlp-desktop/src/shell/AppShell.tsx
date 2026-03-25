// AppShell: root layout with three nested ResizablePanelGroup instances.
// Outer horizontal: IconRail | nav panel | workspace
// Workspace vertical: content (CommandBar + center/config) | drawer
// Content horizontal: center | config panel

import { useStore } from "@tanstack/react-store";
import {
    ResizableHandle,
    ResizablePanel,
    ResizablePanelGroup,
} from "@/components/ui/resizable";
import { useDefaultLayout } from "react-resizable-panels";
import { IconRail } from "./IconRail";
import { NavPanel } from "./NavPanel";
import { CommandBar } from "./CommandBar";
import { ConfigPanel } from "./ConfigPanel";
import { BottomDrawer } from "./BottomDrawer";
import { CommandPalette } from "./CommandPalette";
import { uiStore } from "@/stores/uiStore";
import { useGlobalHotkeys } from "@/hooks/useGlobalHotkeys";
import { AnalyzeView } from "@/views/analyze/AnalyzeView";
import { QueueView } from "@/views/queue/QueueView";
import { HistoryView } from "@/views/history/HistoryView";
import { SettingsView } from "@/views/settings/SettingsView";

export function AppShell() {
    const activeView = useStore(uiStore, (s) => s.activeView);
    useGlobalHotkeys();

    // Persist panel layouts to localStorage
    const outerLayout = useDefaultLayout({ id: "rdlp-outer", storage: localStorage });
    const workspaceLayout = useDefaultLayout({ id: "rdlp-workspace", storage: localStorage });
    const contentLayout = useDefaultLayout({ id: "rdlp-content", storage: localStorage });

    return (
        <div className="flex h-full w-full overflow-hidden bg-[var(--surface-deepest)]">
            {/* Fixed icon rail — not inside a resizable panel */}
            <IconRail />

            {/* Outer horizontal group: nav panel | workspace */}
            <ResizablePanelGroup orientation="horizontal" className="flex-1" defaultLayout={outerLayout.defaultLayout} onLayoutChanged={outerLayout.onLayoutChanged}>
                {/* Nav panel — collapsible */}
                <ResizablePanel
                    id="nav"
                    defaultSize={15}
                    minSize={10}
                    collapsible
                    collapsedSize={0}
                    className="bg-[var(--surface-base)]"
                >
                    <NavPanel />
                </ResizablePanel>

                <ResizableHandle className="bg-[#1a1a2e] w-px hover:bg-[#2a2a3e] transition-colors" />

                {/* Workspace panel — vertical stack */}
                <ResizablePanel id="workspace" defaultSize={85} minSize={50}>
                    <ResizablePanelGroup orientation="vertical" defaultLayout={workspaceLayout.defaultLayout} onLayoutChanged={workspaceLayout.onLayoutChanged}>
                        {/* Content area: CommandBar + horizontal center/config */}
                        <ResizablePanel id="content" defaultSize={93} minSize={60}>
                            <div className="flex flex-col h-full">
                                {/* Top command bar */}
                                <CommandBar />

                                {/* Horizontal content + config panels */}
                                <ResizablePanelGroup orientation="horizontal" className="flex-1 min-h-0" defaultLayout={contentLayout.defaultLayout} onLayoutChanged={contentLayout.onLayoutChanged}>
                                    {/* Center workspace */}
                                    <ResizablePanel
                                        id="center"
                                        defaultSize={75}
                                        minSize={40}
                                        className="overflow-hidden"
                                    >
                                        <div className="h-full overflow-y-auto overflow-x-hidden bg-[var(--surface-raised)]">
                                            {activeView === "analyze" && <AnalyzeView />}
                                            {activeView === "queue" && <QueueView />}
                                            {activeView === "history" && <HistoryView />}
                                            {activeView === "settings" && <SettingsView />}
                                        </div>
                                    </ResizablePanel>

                                    <ResizableHandle className="bg-[#1a1a2e] w-px hover:bg-[#2a2a3e] transition-colors" />

                                    {/* Config panel — collapsible */}
                                    <ResizablePanel
                                        id="config"
                                        defaultSize={25}
                                        minSize={14}
                                        collapsible
                                        collapsedSize={0}
                                        className="bg-[var(--surface-base)]"
                                    >
                                        <ConfigPanel />
                                    </ResizablePanel>
                                </ResizablePanelGroup>
                            </div>
                        </ResizablePanel>

                        <ResizableHandle className="bg-[#1a1a2e] h-px hover:bg-[#2a2a3e] transition-colors" />

                        {/* Bottom drawer */}
                        <ResizablePanel
                            id="drawer"
                            defaultSize={7}
                            minSize={2}
                            collapsible
                            collapsedSize={2}
                            className="bg-[var(--surface-deepest)]"
                        >
                            <BottomDrawer />
                        </ResizablePanel>
                    </ResizablePanelGroup>
                </ResizablePanel>
            </ResizablePanelGroup>

            {/* Command palette overlay */}
            <CommandPalette />
        </div>
    );
}
