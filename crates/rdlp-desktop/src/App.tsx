import { useCallback, useEffect, useRef, useState } from "react";
import { QueryClientProvider, useQuery } from "@tanstack/react-query";
import { useStore } from "@tanstack/react-store";
import { SearchPage } from "./pages/SearchPage";
import { QueuePage } from "./pages/QueuePage";
import { SettingsPage } from "./pages/SettingsPage";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { queryClient } from "./query/queryClient";
import { registerDownloadEvents } from "./events/registerDownloadEvents";
import {
    searchParamsAtom,
    setSearchParam,
} from "./stores/searchParamsStore";
import { searchQueryOptions } from "./api/search";
import { downloadsQueryOptions } from "./api/downloads";
import type { SearchFilter, ViewMode } from "./types";

type Tab = "search" | "queue" | "settings";

const VIEW_MODE_KEY = "rdlp-view-mode";
const SIDEBAR_KEY = "rdlp-sidebar-collapsed";

/** Inner component that uses hooks requiring QueryClientProvider. */
function AppContent() {
    const [activeTab, setActiveTab] = useState<Tab>("search");
    const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
        return localStorage.getItem(SIDEBAR_KEY) === "true";
    });
    const [viewMode, setViewMode] = useState<ViewMode>(() => {
        return (localStorage.getItem(VIEW_MODE_KEY) as ViewMode) || "list";
    });
    const [searchDuration, setSearchDuration] = useState<number | null>(null);

    // --- TanStack Store: search form state ---
    const query = useStore(searchParamsAtom, (s) => s.query);
    const site = useStore(searchParamsAtom, (s) => s.site);
    const filters = useStore(searchParamsAtom, (s) => s.filters);

    // --- TanStack Query: server state ---
    const { isFetching, data: searchData } = useQuery(
        searchQueryOptions(query, site, filters),
    );
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    const results = searchData?.results ?? [];

    const activeCount = jobs.filter(
        (j) => j.status === "pending" || j.status === "running",
    ).length;

    // Persist view mode
    const handleViewModeChange = useCallback((mode: ViewMode) => {
        setViewMode(mode);
        localStorage.setItem(VIEW_MODE_KEY, mode);
    }, []);

    // Persist sidebar state
    const toggleSidebar = useCallback(() => {
        setSidebarCollapsed((prev) => {
            const next = !prev;
            localStorage.setItem(SIDEBAR_KEY, String(next));
            return next;
        });
    }, []);

    // Restore search from sidebar history
    const handleRestoreSearch = useCallback(
        (restoredQuery: string, restoredSite: string, restoredFilters: SearchFilter[]) => {
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
                    queryKey: ["search"],
                    refetchType: "all",
                });
            }, 0);
        },
        [],
    );

    // Track search duration via isFetching transitions
    const searchStartRef = useRef<number | null>(null);
    useEffect(() => {
        if (isFetching) {
            searchStartRef.current = Date.now();
        } else if (searchStartRef.current !== null) {
            setSearchDuration(Date.now() - searchStartRef.current);
            searchStartRef.current = null;
        }
    }, [isFetching]);

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
                setActiveTab("settings");
            }
        };
        document.addEventListener("keydown", handler);
        return () => document.removeEventListener("keydown", handler);
    }, [toggleSidebar, handleViewModeChange, viewMode]);

    return (
        <div className="app">
            <nav className="tab-bar">
                <button
                    className={`tab-button ${activeTab === "search" ? "active" : ""}`}
                    onClick={() => setActiveTab("search")}
                >
                    <svg className="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <circle cx="11" cy="11" r="8" />
                        <path d="M21 21l-4.3-4.3" />
                    </svg>
                    Search
                </button>
                <button
                    className={`tab-button ${activeTab === "queue" ? "active" : ""}`}
                    onClick={() => setActiveTab("queue")}
                >
                    <svg className="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                        <polyline points="7 10 12 15 17 10" />
                        <line x1="12" y1="15" x2="12" y2="3" />
                    </svg>
                    Queue
                    {activeCount > 0 && (
                        <span className="tab-badge">{activeCount}</span>
                    )}
                </button>
                <button
                    className={`tab-button ${activeTab === "settings" ? "active" : ""}`}
                    onClick={() => setActiveTab("settings")}
                >
                    <svg className="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <circle cx="12" cy="12" r="3" />
                        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                    </svg>
                    Settings
                </button>
            </nav>

            <div className="app-body">
                <Sidebar
                    collapsed={sidebarCollapsed}
                    onSwitchToQueue={() => setActiveTab("queue")}
                    onRestoreSearch={handleRestoreSearch}
                />

                <main className="content">
                    {activeTab === "search" && (
                        <SearchPage activeTab={activeTab} viewMode={viewMode} />
                    )}
                    {activeTab === "queue" && <QueuePage />}
                    {activeTab === "settings" && <SettingsPage />}
                </main>
            </div>

            <StatusBar
                viewMode={viewMode}
                onViewModeChange={handleViewModeChange}
                resultCount={results.length}
                searchDuration={searchDuration}
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
