// ConfigPanel: right contextual panel.
// Content depends on active view and selection state.

import { useStore } from "@tanstack/react-store";
import { uiStore } from "@/stores/uiStore";
import { DownloadConfig } from "@/views/analyze/DownloadConfig";
import { JobDetails } from "@/views/queue/JobDetails";

export function ConfigPanel() {
    const activeView = useStore(uiStore, (s) => s.activeView);
    const selectedJobId = useStore(uiStore, (s) => s.selectedJobId);
    const analyzeUrl = useStore(uiStore, (s) => s.analyzeUrl);

    // Settings view hides config panel (uses full width)
    if (activeView === "settings") {
        return null;
    }

    // Analyze: show download config when a format is loaded
    if (activeView === "analyze") {
        if (!analyzeUrl) {
            return (
                <div className="h-full flex items-center justify-center p-4">
                    <p className="text-[11px] text-[#444444] text-center">
                        Paste a URL to see format options
                    </p>
                </div>
            );
        }
        return (
            <div className="h-full border-l border-[#1a1a2e]">
                <DownloadConfig />
            </div>
        );
    }

    // Queue: show job details when selected
    if (activeView === "queue") {
        if (!selectedJobId) {
            return (
                <div className="h-full flex items-center justify-center p-4 border-l border-[#1a1a2e]">
                    <p className="text-[11px] text-[#444444] text-center">
                        Select a job to see details
                    </p>
                </div>
            );
        }
        return (
            <div className="h-full border-l border-[#1a1a2e]">
                <JobDetails />
            </div>
        );
    }

    // History: show job details when selected
    if (activeView === "history") {
        if (!selectedJobId) {
            return (
                <div className="h-full flex items-center justify-center p-4 border-l border-[#1a1a2e]">
                    <p className="text-[11px] text-[#444444] text-center">
                        Select an item to see details
                    </p>
                </div>
            );
        }
        return (
            <div className="h-full border-l border-[#1a1a2e]">
                <JobDetails />
            </div>
        );
    }

    return null;
}
