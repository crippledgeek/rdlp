import { useStore } from "@tanstack/react-store";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { LayoutList, LayoutGrid } from "lucide-react";
import { searchParamsAtom } from "../stores/searchParamsStore";
import { providersQueryOptions, searchInfiniteQueryOptions } from "../api/search";
import { downloadsQueryOptions } from "../api/downloads";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { ViewMode } from "../types";

interface StatusBarProps {
    viewMode: ViewMode;
    onViewModeChange: (mode: ViewMode) => void;
    resultCount: number;
    totalEstimate: number | null;
    searchDuration: number | null;
    onSwitchToQueue: () => void;
}

export function StatusBar({
    viewMode,
    onViewModeChange,
    resultCount,
    totalEstimate,
    searchDuration,
    onSwitchToQueue,
}: StatusBarProps) {
    // --- TanStack Store: search form state ---
    const query = useStore(searchParamsAtom, (s) => s.query);
    const site = useStore(searchParamsAtom, (s) => s.site);
    const filters = useStore(searchParamsAtom, (s) => s.filters);

    // --- TanStack Query: server state ---
    const { data: providers = [] } = useQuery(providersQueryOptions());
    const { isFetching, isFetchingNextPage, isError, isSuccess } = useInfiniteQuery(
        searchInfiniteQueryOptions(query, site, filters),
    );
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    // Initial loading vs. appending more pages
    const isInitialLoading = isFetching && !isFetchingNextPage;

    // Derive status from query state (same pattern as SearchPage)
    const status: "idle" | "loading" | "results" | "empty" | "error" = isInitialLoading
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
        // "Filtering..." when refining results already shown; "Searching..." for initial fetch
        leftText = isSuccess ? "Filtering\u2026" : "Searching\u2026";
    } else if (status === "results") {
        const dur = searchDuration !== null ? ` \u00b7 ${(searchDuration / 1000).toFixed(1)}s` : "";
        const countText =
            totalEstimate !== null && totalEstimate > resultCount
                ? `${resultCount} of ~${totalEstimate} results`
                : `${resultCount} results`;
        leftText = `${displaySite} \u00b7 ${countText}${dur}`;
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
        <div className="flex items-center h-6 px-3 bg-background border-t border-border shrink-0">
            <div className="flex items-center gap-1.5 flex-1 min-w-0">
                {status === "error" && (
                    <span className="w-1.5 h-1.5 rounded-full bg-destructive shrink-0" />
                )}
                <span
                    className={cn(
                        "font-mono text-[11px] truncate",
                        status === "error"
                            ? "text-destructive"
                            : "text-muted-foreground",
                    )}
                >
                    {leftText}
                </span>
            </div>
            <div className="flex items-center justify-center flex-1">
                {centerText && (
                    <button
                        className="font-mono text-[11px] text-primary hover:text-primary/80 bg-transparent border-none cursor-pointer"
                        onClick={onSwitchToQueue}
                        title="Switch to Queue tab"
                    >
                        {centerText}
                    </button>
                )}
            </div>
            <div className="flex items-center gap-0.5 justify-end flex-1">
                <Button
                    variant="ghost"
                    className={cn(
                        "h-[18px] w-[22px] p-0",
                        viewMode === "list"
                            ? "text-foreground"
                            : "text-muted-foreground hover:text-foreground",
                    )}
                    onClick={() => onViewModeChange("list")}
                    title="List view"
                    aria-label="List view"
                >
                    <LayoutList size={14} />
                </Button>
                <Button
                    variant="ghost"
                    className={cn(
                        "h-[18px] w-[22px] p-0",
                        viewMode === "grid"
                            ? "text-foreground"
                            : "text-muted-foreground hover:text-foreground",
                    )}
                    onClick={() => onViewModeChange("grid")}
                    title="Grid view"
                    aria-label="Grid view"
                >
                    <LayoutGrid size={14} />
                </Button>
            </div>
        </div>
    );
}
