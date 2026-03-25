// NavPanel: context-dependent left panel content.
// Renders different content based on the active view.

import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { uiStore } from "@/stores/uiStore";
import { downloadsQueryOptions } from "@/api/downloads";
import { JobMiniCard } from "@/components/JobMiniCard";
import { QueueNav } from "@/views/queue/QueueNav";
import { HistoryNav } from "@/views/history/HistoryNav";
import { SettingsNav } from "@/views/settings/SettingsNav";

function AnalyzeNav() {
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());
    const activeJobs = jobs.filter((j) => j.status === "running" || j.status === "pending");

    return (
        <div className="flex flex-col h-full overflow-y-auto p-2 gap-3">
            {/* Active jobs */}
            {activeJobs.length > 0 && (
                <section>
                    <h2 className="section-heading px-2">Active Downloads</h2>
                    <div className="flex flex-col gap-0.5">
                        {activeJobs.map((job) => (
                            <JobMiniCard key={job.id} job={job} />
                        ))}
                    </div>
                </section>
            )}

            {/* Recent URLs placeholder */}
            <section>
                <h2 className="section-heading px-2">Recent</h2>
                <div className="flex flex-col gap-0.5">
                    <p className="text-[11px] text-[#444444] px-2 py-1">No recent URLs</p>
                </div>
            </section>
        </div>
    );
}



export function NavPanel() {
    const activeView = useStore(uiStore, (s) => s.activeView);

    return (
        <div className="h-full overflow-hidden border-r border-[#1a1a2e]">
            {activeView === "analyze" && <AnalyzeNav />}
            {activeView === "queue" && <QueueNav />}
            {activeView === "history" && <HistoryNav />}
            {activeView === "settings" && <SettingsNav />}
        </div>
    );
}
