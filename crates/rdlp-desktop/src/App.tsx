import { useEffect, useState } from "react";
import { SearchPage } from "./pages/SearchPage";
import { QueuePage } from "./pages/QueuePage";
import { SettingsPage } from "./pages/SettingsPage";
import { useQueueStore } from "./lib/store";
import {
    onDownloadProgress,
    onDownloadComplete,
    onDownloadError,
    onDownloadLog,
    onFormatSelected,
} from "./lib/tauri";

type Tab = "search" | "queue" | "settings";

function App() {
    const [activeTab, setActiveTab] = useState<Tab>("search");
    const jobs = useQueueStore((s) => s.jobs);

    const activeCount = jobs.filter(
        (j) => j.status === "pending" || j.status === "running",
    ).length;

    useEffect(() => {
        let mounted = true;
        const unlisteners: Array<() => void> = [];

        const setup = async () => {
            const unProgress = await onDownloadProgress((payload) => {
                useQueueStore
                    .getState()
                    .updateJobFromProgress(
                        payload.jobId,
                        payload.progress,
                        payload.speed,
                        payload.eta,
                    );
            });
            if (!mounted) { unProgress(); return; }
            unlisteners.push(unProgress);

            const unComplete = await onDownloadComplete((payload) => {
                useQueueStore
                    .getState()
                    .markJobCompleted(payload.jobId, payload.filepath);
            });
            if (!mounted) { unComplete(); return; }
            unlisteners.push(unComplete);

            const unError = await onDownloadError((payload) => {
                useQueueStore
                    .getState()
                    .markJobFailed(
                        payload.jobId,
                        payload.error,
                        payload.retryable,
                    );
            });
            if (!mounted) { unError(); return; }
            unlisteners.push(unError);

            const unLog = await onDownloadLog((payload) => {
                useQueueStore
                    .getState()
                    .updateJobStatus(payload.jobId, payload.message);
            });
            if (!mounted) { unLog(); return; }
            unlisteners.push(unLog);

            const unFormatSelected = await onFormatSelected((payload) => {
                useQueueStore
                    .getState()
                    .updateJobStatus(
                        payload.jobId,
                        `Format: ${payload.quality}`,
                    );
            });
            if (!mounted) { unFormatSelected(); return; }
            unlisteners.push(unFormatSelected);
        };

        void setup();

        return () => {
            mounted = false;
            for (const unlisten of unlisteners) {
                unlisten();
            }
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

            <main className="content">
                {activeTab === "search" && <SearchPage />}
                {activeTab === "queue" && <QueuePage />}
                {activeTab === "settings" && <SettingsPage />}
            </main>
        </div>
    );
}

export default App;
