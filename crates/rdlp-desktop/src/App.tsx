import { useEffect, useState } from "react";
import { QueryClientProvider, useQuery } from "@tanstack/react-query";
import { Search, Download, ArrowDownToLine, Settings } from "lucide-react";
import { SearchPage } from "./pages/SearchPage";
import { QueuePage } from "./pages/QueuePage";
import { SettingsPage } from "./pages/SettingsPage";
import { DownloadPage } from "./pages/DownloadPage";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { queryClient } from "./query/queryClient";
import { registerDownloadEvents } from "./events/registerDownloadEvents";
import {
    searchParamsAtom,
    setSearchParam,
} from "./stores/searchParamsStore";
import { queryKeys } from "./query/queryKeys";
import { downloadsQueryOptions } from "./api/downloads";
import { cn } from "@/lib/utils";
import type { SearchFilter, ViewMode } from "./types";

type Tab = "search" | "queue" | "download" | "settings";

const VIEW_MODE_KEY = "rdlp-view-mode";
const SIDEBAR_KEY = "rdlp-sidebar-collapsed";

const TAB_CONFIG = [
    { id: "search" as const, label: "Search", icon: Search },
    { id: "queue" as const, label: "Queue", icon: Download },
    { id: "download" as const, label: "Download", icon: ArrowDownToLine },
    { id: "settings" as const, label: "Settings", icon: Settings },
] as const;

/** Inner component that uses hooks requiring QueryClientProvider. */
function AppContent() {
    const [activeTab, setActiveTab] = useState<Tab>("search");
    const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
        return localStorage.getItem(SIDEBAR_KEY) === "true";
    });
    const [viewMode, setViewMode] = useState<ViewMode>(() => {
        return (localStorage.getItem(VIEW_MODE_KEY) as ViewMode) || "list";
    });
    // --- TanStack Query: downloads only (search state lives in SearchPage + StatusBar) ---
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    const activeCount = jobs.filter(
        (j) => j.status === "pending" || j.status === "running",
    ).length;

    // Persist view mode
    const handleViewModeChange = (mode: ViewMode) => {
        setViewMode(mode);
        localStorage.setItem(VIEW_MODE_KEY, mode);
    };

    // Persist sidebar state
    const toggleSidebar = () => {
        setSidebarCollapsed((prev) => {
            const next = !prev;
            localStorage.setItem(SIDEBAR_KEY, String(next));
            return next;
        });
    };

    // Restore search from sidebar history
    const handleRestoreSearch = (restoredQuery: string, restoredSite: string, restoredFilters: SearchFilter[]) => {
        setSearchParam("query", restoredQuery);
        setSearchParam("site", restoredSite);
        if (restoredFilters.length > 0) {
            searchParamsAtom.setState((prev) => ({
                ...prev,
                filters: restoredFilters,
                hasUserFilters: true,
            }));
        }
        setActiveTab("search");
        // Invalidate the search query to trigger a re-fetch after atom updates propagate
        setTimeout(() => {
            void queryClient.invalidateQueries({
                queryKey: queryKeys.search.params(restoredQuery, restoredSite, restoredFilters),
                refetchType: "all",
            });
        }, 0);
    };

    // Register all Tauri download event listeners (single wiring point)
    useEffect(() => {
        return registerDownloadEvents(queryClient);
    }, []);

    // Global keyboard shortcuts
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            // Ctrl+B: toggle sidebar
            if ((e.ctrlKey || e.metaKey) && e.key === "b") {
                e.preventDefault();
                toggleSidebar();
            }
            // Ctrl+K: focus search input
            if ((e.ctrlKey || e.metaKey) && e.key === "k") {
                e.preventDefault();
                setActiveTab("search");
                // Dispatch custom event for CommandBar to pick up
                window.dispatchEvent(new CustomEvent("rdlp-focus-search"));
            }
            // Ctrl+L: toggle view mode
            if ((e.ctrlKey || e.metaKey) && e.key === "l") {
                e.preventDefault();
                handleViewModeChange(viewMode === "list" ? "grid" : "list");
            }
            // Ctrl+1/2/3: switch tabs
            if ((e.ctrlKey || e.metaKey) && e.key === "1") {
                e.preventDefault();
                setActiveTab("search");
            }
            if ((e.ctrlKey || e.metaKey) && e.key === "2") {
                e.preventDefault();
                setActiveTab("queue");
            }
            if ((e.ctrlKey || e.metaKey) && e.key === "3") {
                e.preventDefault();
                setActiveTab("download");
            }
            if ((e.ctrlKey || e.metaKey) && e.key === "4") {
                e.preventDefault();
                setActiveTab("settings");
            }
            if ((e.ctrlKey || e.metaKey) && e.key === "d") {
                e.preventDefault();
                setActiveTab("download");
                window.dispatchEvent(new CustomEvent("rdlp-focus-url"));
            }
        };
        document.addEventListener("keydown", handler);
        return () => document.removeEventListener("keydown", handler);
    }, [toggleSidebar, handleViewModeChange, viewMode]);

    return (
        <div className="flex flex-col h-full bg-background">
            <nav
                className="flex items-center gap-0.5 px-3 bg-background border-b border-border shrink-0 h-12"
                style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
            >
                {TAB_CONFIG.map(({ id, label, icon: Icon }) => (
                    <button
                        key={id}
                        className={cn(
                            "flex items-center gap-1.5 px-3 h-full border-none bg-transparent text-muted-foreground text-[13px] font-medium tracking-wide cursor-pointer relative transition-colors select-none",
                            "hover:text-foreground/70",
                            activeTab === id && "text-foreground",
                        )}
                        style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
                        onClick={() => setActiveTab(id)}
                    >
                        <Icon className={cn("h-4 w-4 opacity-60 transition-opacity", activeTab === id && "opacity-100")} />
                        {label}
                        {id === "queue" && activeCount > 0 && (
                            <span className="min-w-[18px] h-[18px] px-[5px] rounded-full bg-primary text-background text-[10px] font-bold font-mono flex items-center justify-center">
                                {activeCount}
                            </span>
                        )}
                        {activeTab === id && (
                            <span className="absolute bottom-0 left-3 right-3 h-0.5 bg-primary rounded-t-sm animate-in fade-in zoom-in-x-0 duration-200" />
                        )}
                    </button>
                ))}
            </nav>

            <div className="flex flex-1 overflow-hidden">
                <Sidebar
                    collapsed={sidebarCollapsed}
                    onSwitchToQueue={() => setActiveTab("queue")}
                    onRestoreSearch={handleRestoreSearch}
                />

                <main className="flex-1 overflow-y-auto overflow-x-hidden p-4 scroll-smooth min-w-0">
                    {activeTab === "search" && (
                        <SearchPage activeTab={activeTab} viewMode={viewMode} />
                    )}
                    {activeTab === "queue" && <QueuePage />}
                    {activeTab === "download" && <DownloadPage />}
                    {activeTab === "settings" && <SettingsPage />}
                </main>
            </div>

            <StatusBar
                viewMode={viewMode}
                onViewModeChange={handleViewModeChange}
                onSwitchToQueue={() => setActiveTab("queue")}
            />
        </div>
    );
}

/** Root component — provides QueryClient to the entire tree. */
function App() {
    return (
        <QueryClientProvider client={queryClient}>
            <AppContent />
        </QueryClientProvider>
    );
}

export default App;
