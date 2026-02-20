import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { downloadsQueryOptions } from "../api/downloads";

interface SidebarDownloadsProps {
    onSwitchToQueue: () => void;
}

export function SidebarDownloads({ onSwitchToQueue }: SidebarDownloadsProps) {
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());
    const [collapsed, setCollapsed] = useState(false);

    const sorted = [...jobs].sort((a, b) => {
        const order: Record<string, number> = {
            running: 0, pending: 1, failed: 2, completed: 3, cancelled: 4,
        };
        return (order[a.status] ?? 5) - (order[b.status] ?? 5);
    }).slice(0, 5);

    const activeCount = jobs.filter(
        (j) => j.status === "pending" || j.status === "running",
    ).length;

    return (
        <div className="sidebar-section">
            <button
                className="sidebar-section-header"
                onClick={() => setCollapsed(!collapsed)}
                aria-expanded={!collapsed}
            >
                <svg
                    className={`sidebar-chevron ${collapsed ? "collapsed" : ""}`}
                    viewBox="0 0 16 16" width="10" height="10"
                    fill="currentColor"
                >
                    <path d="M4 6l4 4 4-4" />
                </svg>
                <span>DOWNLOADS</span>
                {activeCount > 0 && (
                    <span className="sidebar-badge">{activeCount}</span>
                )}
            </button>

            {!collapsed && (
                <div className="sidebar-section-content">
                    {sorted.length === 0 ? (
                        <div className="sidebar-empty">No downloads</div>
                    ) : (
                        sorted.map((job) => (
                            <button
                                key={job.id}
                                className="sidebar-download-item"
                                onClick={onSwitchToQueue}
                                title={job.title ?? job.url}
                            >
                                <div className="sidebar-download-row">
                                    {job.status === "completed" && (
                                        <span className="sidebar-status-icon completed">&#x2713;</span>
                                    )}
                                    {job.status === "failed" && (
                                        <span className="sidebar-status-icon failed">&#x2717;</span>
                                    )}
                                    {(job.status === "running" || job.status === "pending") && (
                                        <span className="sidebar-status-icon running" />
                                    )}
                                    <span className="sidebar-download-title">
                                        {job.title ?? "Untitled"}
                                    </span>
                                </div>

                                {(job.status === "running" || job.status === "pending") && (
                                    <>
                                        <div className="sidebar-progress-track">
                                            <div
                                                className="sidebar-progress-fill"
                                                style={{ width: `${(job.progress ?? 0) * 100}%` }}
                                            />
                                        </div>
                                        <div className="sidebar-download-meta">
                                            {job.speed && <span>{job.speed}</span>}
                                            {job.eta && <span>&middot; {job.eta}</span>}
                                        </div>
                                    </>
                                )}
                            </button>
                        ))
                    )}
                </div>
            )}
        </div>
    );
}
