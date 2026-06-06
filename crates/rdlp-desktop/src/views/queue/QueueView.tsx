// QueueView: virtualized job list for the Queue view.
// Groups playlist jobs under collapsible headers. Standalone jobs render flat.
// Uses flattened QueueItem[] array for TanStack Virtual (same pattern as FormatsTable).

import { useRef, useMemo, useState, useCallback } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Download } from "lucide-react";
import { downloadsQueryOptions } from "@/api/downloads";
import { queueFilterStore } from "@/stores/queueFilterStore";
import { jobMatchesFilter, isDoneNotFailed, type QueueFilter } from "@/lib/jobStatus";
import { PlaylistGroupHeader } from "./PlaylistGroupHeader";
import { JobCard } from "./JobCard";
import type { DownloadJob, QueueItem } from "@/types";

const STATUS_ORDER: Record<string, number> = {
    running: 0,
    processing: 1,
    pending: 2,
    failed: 3,
    completed: 4,
    cancelled: 5,
};

/** Height constants for estimateSize callback. */
const GROUP_HEADER_HEIGHT = 64;
const COMPACT_JOB_HEIGHT = 72;
const STANDALONE_JOB_HEIGHT = 90;
const SHOW_MORE_HEIGHT = 32;

function filterJobs(jobs: DownloadJob[], filter: QueueFilter): DownloadJob[] {
    return jobs.filter((j) => jobMatchesFilter(j.status, filter));
}

/**
 * Build a flat QueueItem array from jobs, grouping by playlistId.
 *
 * - Playlist groups: group-header, then job items (sorted by playlistIndex), then optional show-more
 * - Standalone jobs: flat job items
 * - Groups sorted: active-first, then by most recent started_at
 * - Standalone jobs sorted by status priority, then by started_at
 */
function buildFlatItems(
    jobs: DownloadJob[],
    collapsedMap: Map<string, boolean>,
): QueueItem[] {
    // Separate playlist vs standalone
    const playlistGroups = new Map<string, DownloadJob[]>();
    const standaloneJobs: DownloadJob[] = [];

    for (const job of jobs) {
        if (job.playlist) {
            const id = job.playlist.playlistId;
            const group = playlistGroups.get(id);
            if (group) {
                group.push(job);
            } else {
                playlistGroups.set(id, [job]);
            }
        } else {
            standaloneJobs.push(job);
        }
    }

    // Sort groups: active-first (has running/pending), then by most recent started_at
    const sortedGroups = [...playlistGroups.entries()].sort(([, aJobs], [, bJobs]) => {
        const aHasActive = aJobs.some((j) => j.status === "running" || j.status === "pending");
        const bHasActive = bJobs.some((j) => j.status === "running" || j.status === "pending");
        if (aHasActive !== bHasActive) return aHasActive ? -1 : 1;

        const aLatest = Math.max(...aJobs.map((j) => j.started_at ?? 0));
        const bLatest = Math.max(...bJobs.map((j) => j.started_at ?? 0));
        return bLatest - aLatest;
    });

    // Sort standalone by status priority, then by started_at
    const sortedStandalone = [...standaloneJobs].sort((a, b) => {
        const aOrder = STATUS_ORDER[a.status] ?? 5;
        const bOrder = STATUS_ORDER[b.status] ?? 5;
        if (aOrder !== bOrder) return aOrder - bOrder;
        return (b.started_at ?? 0) - (a.started_at ?? 0);
    });

    const items: QueueItem[] = [];

    // Emit playlist groups
    for (const [playlistId, groupJobs] of sortedGroups) {
        // Sort within group by playlistIndex
        const sorted = [...groupJobs].sort(
            (a, b) => (a.playlist?.playlistIndex ?? 0) - (b.playlist?.playlistIndex ?? 0),
        );

        const playlistTitle = sorted[0]?.playlist?.playlistTitle ?? "Playlist";
        const allDone = sorted.every((j) => isDoneNotFailed(j.status));
        const collapsed = collapsedMap.get(playlistId) ?? allDone;

        items.push({
            type: "group-header",
            playlistId,
            playlistTitle,
            jobs: sorted,
            collapsed,
        });

        if (!collapsed) {
            for (const job of sorted) {
                items.push({ type: "job", job, inPlaylist: true });
            }
        }
    }

    // Emit standalone jobs
    for (const job of sortedStandalone) {
        items.push({ type: "job", job, inPlaylist: false });
    }

    return items;
}

export function QueueView() {
    const parentRef = useRef<HTMLDivElement>(null);
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());
    const activeFilter = useStore(queueFilterStore, (s) => s.filter);
    const [collapsedMap, setCollapsedMap] = useState<Map<string, boolean>>(new Map());

    const filtered = useMemo(() => filterJobs(jobs, activeFilter), [jobs, activeFilter]);

    const flatItems = useMemo(
        () => buildFlatItems(filtered, collapsedMap),
        [filtered, collapsedMap],
    );

    const handleToggleCollapse = useCallback((playlistId: string) => {
        setCollapsedMap((prev) => {
            const next = new Map(prev);
            next.set(playlistId, !prev.get(playlistId));
            return next;
        });
    }, []);

    const virtualizer = useVirtualizer({
        count: flatItems.length,
        getScrollElement: () => parentRef.current,
        estimateSize: (i) => {
            const item = flatItems[i];
            if (!item) return STANDALONE_JOB_HEIGHT;
            switch (item.type) {
                case "group-header":
                    return GROUP_HEADER_HEIGHT;
                case "job":
                    return item.inPlaylist ? COMPACT_JOB_HEIGHT : STANDALONE_JOB_HEIGHT;
                case "show-more":
                    return SHOW_MORE_HEIGHT;
            }
        },
        overscan: 5,
    });

    if (jobs.length === 0) {
        return (
            <div className="flex flex-col items-center justify-center h-full gap-3">
                <Download className="w-10 h-10 text-[#1a1a2e]" />
                <p className="text-[13px] text-[var(--text-muted)]">No downloads yet</p>
                <p className="text-[11px] text-[var(--text-muted)]">Paste a URL in the command bar to start</p>
            </div>
        );
    }

    if (flatItems.length === 0) {
        return (
            <div className="flex items-center justify-center h-full">
                <p className="text-[13px] text-[var(--text-muted)]">No jobs match this filter</p>
            </div>
        );
    }

    return (
        <div ref={parentRef} className="h-full overflow-y-auto">
            <div
                className="relative p-3"
                style={{ height: virtualizer.getTotalSize() + 24 }}
            >
                {virtualizer.getVirtualItems().map((virtualItem) => {
                    const item = flatItems[virtualItem.index];
                    if (!item) return null;

                    return (
                        <div
                            key={
                                item.type === "group-header"
                                    ? `group-${item.playlistId}`
                                    : item.type === "job"
                                      ? item.job.id
                                      : `more-${item.playlistId}`
                            }
                            className="absolute left-3 right-3"
                            style={{
                                top: virtualItem.start + 12,
                                height: virtualItem.size,
                            }}
                        >
                            {item.type === "group-header" && (
                                <PlaylistGroupHeader
                                    playlistId={item.playlistId}
                                    playlistTitle={item.playlistTitle}
                                    jobs={item.jobs}
                                    collapsed={item.collapsed}
                                    onToggleCollapse={handleToggleCollapse}
                                />
                            )}
                            {item.type === "job" && (
                                <JobCard job={item.job} compact={item.inPlaylist} />
                            )}
                            {item.type === "show-more" && (
                                <button
                                    onClick={() => handleToggleCollapse(item.playlistId)}
                                    className="w-full text-center text-[11px] text-[#4a9eff] hover:text-[#3a8ef0] py-1 cursor-pointer transition-colors"
                                >
                                    Show {item.hiddenCount} more
                                </button>
                            )}
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
