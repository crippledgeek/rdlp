import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { searchParamsAtom } from "../stores/searchParamsStore";
import { providersQueryOptions, searchQueryOptions } from "../api/search";
import { downloadsQueryOptions } from "../api/downloads";
import type { ViewMode } from "../types";

interface StatusBarProps {
    viewMode: ViewMode;
    onViewModeChange: (mode: ViewMode) => void;
    resultCount: number;
    searchDuration: number | null;
    onSwitchToQueue: () => void;
}

export function StatusBar({
    viewMode,
    onViewModeChange,
    resultCount,
    searchDuration,
    onSwitchToQueue,
}: StatusBarProps) {
    // --- TanStack Store: search form state ---
    const query = useStore(searchParamsAtom, (s) => s.query);
    const site = useStore(searchParamsAtom, (s) => s.site);
    const filters = useStore(searchParamsAtom, (s) => s.filters);

    // --- TanStack Query: server state ---
    const { data: providers = [] } = useQuery(providersQueryOptions());
    const { isFetching, isError, isSuccess } = useQuery(
        searchQueryOptions(query, site, filters),
    );
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    // Derive status from query state (same pattern as SearchPage)
    const status: "idle" | "loading" | "results" | "empty" | "error" = isFetching
        ? "loading"
        : isError
            ? "error"
            : isSuccess && resultCount === 0
                ? "empty"
                : isSuccess
                    ? "results"
                    : "idle";

    const activeJobs = jobs.filter(
        (j) => j.status === "pending" || j.status === "running",
    );
    const activeCount = activeJobs.length;
    const displaySite =
        providers.find((p) => p.name === site)?.display_name ?? site;

    let leftText = "Ready";
    if (status === "loading") {
        leftText = "Searching\u2026";
    } else if (status === "results") {
        const dur = searchDuration !== null ? ` \u00b7 ${(searchDuration / 1000).toFixed(1)}s` : "";
        leftText = `${displaySite} \u00b7 ${resultCount} results${dur}`;
    } else if (status === "empty") {
        leftText = `${displaySite} \u00b7 No results`;
    } else if (status === "error") {
        leftText = "Search failed";
    }

    let centerText = "";
    if (activeCount > 0) {
        const fastest = activeJobs.find((j) => j.speed);
        centerText = fastest?.speed
            ? `\u2193${activeCount} at ${fastest.speed}`
            : `\u2193${activeCount} active`;
    }

    return (
        <div className="status-bar">
            <div className="status-bar-left">
                {status === "error" && (
                    <span className="status-bar-dot status-bar-dot-error" />
                )}
                <span className={status === "error" ? "status-bar-error-text" : ""}>
                    {leftText}
                </span>
            </div>
            <div className="status-bar-center">
                {centerText && (
                    <button
                        className="status-bar-downloads"
                        onClick={onSwitchToQueue}
                        title="Switch to Queue tab"
                    >
                        {centerText}
                    </button>
                )}
            </div>
            <div className="status-bar-right">
                <button
                    className={`status-bar-view-btn ${viewMode === "list" ? "active" : ""}`}
                    onClick={() => onViewModeChange("list")}
                    title="List view"
                    aria-label="List view"
                >
                    <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
                        <rect x="1" y="2" width="14" height="1.5" rx="0.5" />
                        <rect x="1" y="7" width="14" height="1.5" rx="0.5" />
                        <rect x="1" y="12" width="14" height="1.5" rx="0.5" />
                    </svg>
                </button>
                <button
                    className={`status-bar-view-btn ${viewMode === "grid" ? "active" : ""}`}
                    onClick={() => onViewModeChange("grid")}
                    title="Grid view"
                    aria-label="Grid view"
                >
                    <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
                        <rect x="1" y="1" width="6" height="6" rx="1" />
                        <rect x="9" y="1" width="6" height="6" rx="1" />
                        <rect x="1" y="9" width="6" height="6" rx="1" />
                        <rect x="9" y="9" width="6" height="6" rx="1" />
                    </svg>
                </button>
            </div>
        </div>
    );
}
