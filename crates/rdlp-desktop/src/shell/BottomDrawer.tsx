// BottomDrawer: status bar (always visible) + tabbed expansion with Logs/Jobs tabs.
// Uses the bottom ResizablePanel's collapsible behavior for expand/collapse.

import { useState, useCallback } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { ChevronUp, ChevronDown } from "lucide-react";
import { Tabs, TabList, Tab, TabPanel } from "@/components/ui/tabs";
import { uiStore, toggleBottomDrawer, setSelectedJob, setView } from "@/stores/uiStore";
import { downloadsQueryOptions } from "@/api/downloads";
import { LogViewer, appendLog } from "@/components/LogViewer";
import { StatusBadge } from "@/components/StatusBadge";
import type { DownloadJob } from "@/types";

// Re-export appendLog so event registration can use it
export { appendLog };

interface JobsPanelProps {
    jobs: DownloadJob[];
}

function JobsPanel({ jobs }: JobsPanelProps) {
    const handleJobClick = useCallback((jobId: string) => {
        setView("queue");
        setSelectedJob(jobId);
    }, []);

    if (jobs.length === 0) {
        return <p className="text-[11px] text-[#444444]">No jobs</p>;
    }
    return (
        <div className="flex flex-col gap-1">
            {jobs.map((job) => (
                <button
                    key={job.id}
                    onClick={() => handleJobClick(job.id)}
                    className="flex items-center gap-2 text-left w-full px-1 py-0.5 rounded-[3px] hover:bg-[#141428] transition-colors"
                >
                    <span className="text-[11px] text-[#aaaaaa] truncate flex-1 min-w-0">
                        {job.playlist
                            ? `Ep ${job.playlist.playlistIndex} \u2014 ${job.title ?? "Untitled"}`
                            : (job.title ?? job.url)}
                    </span>
                    <StatusBadge status={job.status} className="shrink-0" />
                    {(job.status === "running" || job.status === "pending") && job.progress !== null && (
                        <span className="text-[10px] text-[#666666] font-mono shrink-0">
                            {Math.round(job.progress)}%
                        </span>
                    )}
                </button>
            ))}
        </div>
    );
}

export function BottomDrawer() {
    const expanded = useStore(uiStore, (s) => s.bottomDrawerExpanded);
    const [activeTab, setActiveTab] = useState<string>("logs");
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    const activeJob = jobs.find((j) => j.status === "running");
    const activeCount = jobs.filter((j) => j.status === "running" || j.status === "pending").length;

    return (
        <Tabs
            selectedKey={activeTab}
            onSelectionChange={(key) => setActiveTab(String(key))}
            className="h-full flex flex-col bg-[var(--surface-deepest)] border-t border-[#1a1a2e]"
        >
            {/* IDE-style panel header: tabs + status in one row */}
            <div className="flex items-center h-7 px-1 shrink-0 border-b border-[#1a1a2e] bg-[#0a0a14]">
                {/* Tab buttons */}
                <TabList aria-label="Panel tabs" className="flex items-center gap-0 h-7 rounded-none justify-start">
                    <Tab
                        id="logs"
                        className="h-7 rounded-none px-3 py-0 text-[11px] border-b-2 border-transparent data-[selected]:border-[#4a9eff] data-[selected]:text-[#4a9eff] text-[#666666] hover:text-[#aaaaaa] transition-colors"
                    >
                        Logs
                    </Tab>
                    <Tab
                        id="jobs"
                        className="h-7 rounded-none px-3 py-0 text-[11px] border-b-2 border-transparent data-[selected]:border-[#4a9eff] data-[selected]:text-[#4a9eff] text-[#666666] hover:text-[#aaaaaa] transition-colors flex items-center gap-1"
                    >
                        Jobs
                        {activeCount > 0 && (
                            <span className="min-w-[14px] h-[14px] px-[3px] rounded-full bg-[#4a9eff] text-white text-[9px] font-bold flex items-center justify-center">
                                {activeCount}
                            </span>
                        )}
                    </Tab>
                </TabList>

                {/* Active job status — right side */}
                <div className="flex-1 min-w-0 flex items-center justify-end gap-2 px-2">
                    {activeJob ? (
                        <>
                            <span className="w-1.5 h-1.5 rounded-full bg-[#e8a838] animate-pulse shrink-0" />
                            <span className="text-[10px] text-[#666666] truncate">
                                {activeJob.playlist
                                    ? `${activeJob.playlist.playlistTitle} ${activeJob.playlist.playlistIndex}/${activeJob.playlist.playlistCount} \u2014 ${activeJob.title ?? "Untitled"}`
                                    : (activeJob.title ?? activeJob.url)}
                            </span>
                            {activeJob.progress !== null && (
                                <span className="text-[10px] text-[#aaaaaa] font-mono shrink-0">
                                    {Math.round(activeJob.progress)}%
                                </span>
                            )}
                            {activeJob.speed && (
                                <span className="text-[10px] text-[#555555] font-mono shrink-0">
                                    {activeJob.speed}
                                </span>
                            )}
                        </>
                    ) : (
                        <>
                            <span className="w-1.5 h-1.5 rounded-full bg-[#4a9e4a] shrink-0" />
                            <span className="text-[10px] text-[#555555]">Ready</span>
                        </>
                    )}
                </div>

                {/* Expand/collapse toggle */}
                <button
                    onClick={toggleBottomDrawer}
                    aria-label={expanded ? "Collapse drawer" : "Expand drawer"}
                    className="text-[#444444] hover:text-[#aaaaaa] transition-colors px-1"
                >
                    {expanded ? <ChevronDown className="w-3.5 h-3.5" /> : <ChevronUp className="w-3.5 h-3.5" />}
                </button>
            </div>

            {/* Panel content */}
            <div className="flex-1 overflow-hidden min-h-0">
                <TabPanel id="logs" className="h-full overflow-hidden mt-0">
                    <LogViewer />
                </TabPanel>

                <TabPanel id="jobs" className="h-full overflow-y-auto p-2 mt-0">
                    <JobsPanel jobs={jobs} />
                </TabPanel>
            </div>
        </Tabs>
    );
}
