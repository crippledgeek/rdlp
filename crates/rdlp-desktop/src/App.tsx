import { useCallback, useEffect, useState } from "react";
import { SearchPage } from "./pages/SearchPage";
import { QueuePage } from "./pages/QueuePage";
import { SettingsPage } from "./pages/SettingsPage";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { useQueueStore, useSearchStore } from "./lib/store";
import {
    onDownloadProgress,
    onDownloadComplete,
    onDownloadError,
    onDownloadLog,
    onFormatSelected,
} from "./lib/tauri";
import type { SearchFilter, ViewMode } from "./types";

type Tab = "search" | "queue" | "settings";

const VIEW_MODE_KEY = "rdlp-view-mode";
const SIDEBAR_KEY = "rdlp-sidebar-collapsed";

function App() {
    const [activeTab, setActiveTab] = useState<Tab>("search");
    const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
        return localStorage.getItem(SIDEBAR_KEY) === "true";
    });
    const [viewMode, setViewMode] = useState<ViewMode>(() => {
        return (localStorage.getItem(VIEW_MODE_KEY) as ViewMode) || "list";
    });
    const [searchDuration, setSearchDuration] = useState<number | null>(null);

    const jobs = useQueueStore((s) => s.jobs);
    const results = useSearchStore((s) => s.results);
    const status = useSearchStore((s) => s.status);

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
            const store = useSearchStore.getState();
            store.setQuery(restoredQuery);
            store.setSite(restoredSite);
            if (restoredFilters.length > 0) store.setFilters(restoredFilters);
            setActiveTab("search");
            setTimeout(() => { void store.search(); }, 0);
        },
        [],
    );

    // Track search duration
    useEffect(() => {
        if (status === "loading") {
            const start = Date.now();
            const unsub = useSearchStore.subscribe((state) => {
                if (state.status !== "loading") {
                    setSearchDuration(Date.now() - start);
                    unsub();
                }
            });
            return unsub;
        }
    }, [status]);

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

    // Tauri event listeners (unchanged from original)
    useEffect(() => {
        let mounted = true;
        const unlisteners: Array<() => void> = [];

        const setup = async () => {
            const unProgress = await onDownloadProgress((payload) => {
                useQueueStore.getState().updateJobFromProgress(
                    payload.jobId, payload.progress, payload.speed, payload.eta,
                );
            });
            if (!mounted) { unProgress(); return; }
            unlisteners.push(unProgress);

            const unComplete = await onDownloadComplete((payload) => {
                useQueueStore.getState().markJobCompleted(payload.jobId, payload.filepath);
            });
            if (!mounted) { unComplete(); return; }
            unlisteners.push(unComplete);

            const unError = await onDownloadError((payload) => {
                useQueueStore.getState().markJobFailed(payload.jobId, payload.error, payload.retryable);
            });
            if (!mounted) { unError(); return; }
            unlisteners.push(unError);

            const unLog = await onDownloadLog((payload) => {
                useQueueStore.getState().updateJobStatus(payload.jobId, payload.message);
            });
            if (!mounted) { unLog(); return; }
            unlisteners.push(unLog);

            const unFormatSelected = await onFormatSelected((payload) => {
                useQueueStore.getState().updateJobStatus(payload.jobId, `Format: ${payload.quality}`);
            });
            if (!mounted) { unFormatSelected(); return; }
            unlisteners.push(unFormatSelected);
        };

        void setup();
        return () => {
            mounted = false;
            for (const unlisten of unlisteners) unlisten();
        };
    }, []);

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

export default App;
