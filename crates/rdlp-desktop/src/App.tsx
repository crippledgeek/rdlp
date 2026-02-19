import { useEffect, useState } from "react";
import { SearchPage } from "./pages/SearchPage";
import { QueuePage } from "./pages/QueuePage";
import { SettingsPage } from "./pages/SettingsPage";
import { useQueueStore } from "./lib/store";
import {
    onDownloadProgress,
    onDownloadComplete,
    onDownloadError,
} from "./lib/tauri";

type Tab = "search" | "queue" | "settings";

function App() {
    const [activeTab, setActiveTab] = useState<Tab>("search");

    useEffect(() => {
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
            unlisteners.push(unProgress);

            const unComplete = await onDownloadComplete((payload) => {
                useQueueStore
                    .getState()
                    .markJobCompleted(payload.jobId, payload.filepath);
            });
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
            unlisteners.push(unError);
        };

        void setup();

        return () => {
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
                    Search
                </button>
                <button
                    className={`tab-button ${activeTab === "queue" ? "active" : ""}`}
                    onClick={() => setActiveTab("queue")}
                >
                    Queue
                </button>
                <button
                    className={`tab-button ${activeTab === "settings" ? "active" : ""}`}
                    onClick={() => setActiveTab("settings")}
                >
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
