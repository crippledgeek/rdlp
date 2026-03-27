// EpisodeList: virtualized multi-select episode list for playlist URLs.
// Uses Jolly GridList for selection + keyboard nav, TanStack Virtual for DOM windowing.

import { useRef, useMemo, useCallback } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { type Selection } from "react-aria-components";
import { useState } from "react";
import { Download } from "lucide-react";
import { GridList, GridListItem } from "@/components/ui/grid-list";
import { Toolbar } from "@/components/ui/toolbar";
import { Checkbox } from "@/components/ui/checkbox";
import { StreamBadge } from "@/components/StreamBadge";
import { Thumbnail } from "@/components/Thumbnail";
import { setAnalyzeUrl } from "@/stores/uiStore";
import { uiStore } from "@/stores/uiStore";
import type { PlaylistEntry } from "@/types";
import { cn } from "@/lib/utils";

interface EpisodeListProps {
    episodes: PlaylistEntry[];
    playlistUrl: string;
}

function formatDuration(seconds: number): string {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    if (h > 0) {
        return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    }
    return `${m}:${String(s).padStart(2, "0")}`;
}

const ROW_HEIGHT = 48;
const OVERSCAN = 10;

export function EpisodeList({ episodes, playlistUrl }: EpisodeListProps) {
    const parentRef = useRef<HTMLDivElement>(null);
    const [selectedKeys, setSelectedKeys] = useState<Selection>(new Set<string>());

    const selectedCount = useMemo(() => {
        if (selectedKeys === "all") return episodes.length;
        return selectedKeys.size;
    }, [selectedKeys, episodes.length]);

    const isAllSelected = selectedCount === episodes.length && episodes.length > 0;
    const isIndeterminate = selectedCount > 0 && selectedCount < episodes.length;

    const virtualizer = useVirtualizer({
        count: episodes.length,
        getScrollElement: () => parentRef.current,
        estimateSize: () => ROW_HEIGHT,
        overscan: OVERSCAN,
    });

    const handleSelectAll = useCallback(
        (isSelected: boolean) => {
            if (isSelected) {
                setSelectedKeys("all");
            } else {
                setSelectedKeys(new Set<string>());
            }
        },
        [],
    );

    const handleRowAction = useCallback(
        (key: React.Key) => {
            const ep = episodes.find((e) => e.url === String(key));
            if (!ep) return;
            // Store current playlist URL for back navigation
            uiStore.setState((prev) => ({
                ...prev,
                playlistBackUrl: playlistUrl,
            }));
            setAnalyzeUrl(ep.url);
        },
        [episodes, playlistUrl],
    );

    const handleDownloadSelected = useCallback(() => {
        // Gather selected episode URLs for batch download (wired in SP4)
        const _selected =
            selectedKeys === "all"
                ? episodes
                : episodes.filter((ep) => (selectedKeys as Set<string>).has(ep.url));
        // TODO(SP4): invoke start_download for each selected episode with PlaylistContext
        void _selected;
    }, [selectedKeys, episodes]);

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Selection toolbar */}
            <div className="flex items-center gap-3 px-3 py-1.5 border-b border-[#1a1a2e] bg-[var(--surface-raised)] shrink-0">
                <Toolbar aria-label="Episode selection" className="flex items-center gap-3 flex-1">
                    <Checkbox
                        isSelected={isAllSelected}
                        isIndeterminate={isIndeterminate}
                        onChange={handleSelectAll}
                        aria-label={isAllSelected ? "Deselect all episodes" : "Select all episodes"}
                        className="text-[12px]"
                    >
                        <span className="text-[11px] text-[#888888]">
                            {isAllSelected ? "Deselect All" : "Select All"}
                        </span>
                    </Checkbox>

                    <div className="flex-1" />

                    <span className="text-[11px] text-[#666666] tabular-nums" aria-live="polite">
                        {selectedCount > 0
                            ? `${selectedCount} of ${episodes.length} selected`
                            : `${episodes.length} episodes`}
                    </span>

                    <button
                        onClick={handleDownloadSelected}
                        disabled={selectedCount === 0}
                        className={cn(
                            "flex items-center gap-1.5 px-3 py-1 rounded-[6px] text-[12px] font-medium transition-colors cursor-pointer",
                            selectedCount > 0
                                ? "bg-[#4a9eff] text-white hover:bg-[#3a8ef0]"
                                : "bg-[#1a1a2e] text-[#444444] cursor-not-allowed pointer-events-none",
                        )}
                    >
                        <Download className="w-3 h-3" />
                        Download Selected
                    </button>
                </Toolbar>
            </div>

            {/* Virtualized episode list */}
            <div ref={parentRef} className="flex-1 overflow-y-auto">
                <GridList
                    aria-label="Episodes"
                    selectionMode="multiple"
                    selectionBehavior="toggle"
                    selectedKeys={selectedKeys}
                    onSelectionChange={setSelectedKeys}
                    onAction={handleRowAction}
                    className="p-0 border-0 rounded-none shadow-none bg-transparent gap-0"
                    style={{
                        height: virtualizer.getTotalSize(),
                        position: "relative",
                    }}
                >
                    {virtualizer.getVirtualItems().map((vItem) => {
                        const ep = episodes[vItem.index];
                        if (!ep) return null;
                        return (
                            <GridListItem
                                key={ep.url}
                                id={ep.url}
                                textValue={ep.title}
                                className={cn(
                                    "flex items-center gap-3 px-3 cursor-pointer rounded-none border-b border-[#1a1a2e]/50",
                                    "data-[hovered]:bg-[#1a1a2e]/60",
                                    "data-[selected]:bg-[#1a2a4a]/40",
                                    "data-[focus-visible]:ring-1 data-[focus-visible]:ring-[#4a9eff] data-[focus-visible]:ring-inset",
                                )}
                                style={{
                                    position: "absolute",
                                    top: vItem.start,
                                    left: 0,
                                    width: "100%",
                                    height: ROW_HEIGHT,
                                }}
                            >
                                {/* Checkbox auto-rendered by GridListItem for selectionMode="multiple" + selectionBehavior="toggle" */}

                                {/* Index */}
                                <span className="w-8 text-right text-[12px] text-[#888888] tabular-nums shrink-0">
                                    {ep.index}
                                </span>

                                {/* Thumbnail */}
                                <div className="w-16 h-9 shrink-0 rounded-[4px] overflow-hidden bg-[#0e0e1e]">
                                    <Thumbnail
                                        src={ep.thumbnailUrl}
                                        alt={ep.title}
                                        className="w-full h-full object-cover"
                                        decoding="async"
                                    />
                                </div>

                                {/* Title */}
                                <span
                                    className="flex-1 min-w-0 text-[13px] text-[#F8FAFC] truncate"
                                >
                                    {ep.title}
                                </span>

                                {/* Duration */}
                                {ep.duration !== null && (
                                    <span className="text-[11px] text-[#888888] tabular-nums shrink-0">
                                        {formatDuration(ep.duration)}
                                    </span>
                                )}

                                {/* SUB/DUB badges */}
                                <div className="flex items-center gap-1 shrink-0">
                                    {ep.hasSub && (
                                        <StreamBadge value="SUB" category="feature" />
                                    )}
                                    {ep.hasDub && (
                                        <StreamBadge value="DUB" category="feature" />
                                    )}
                                </div>
                            </GridListItem>
                        );
                    })}
                </GridList>
            </div>
        </div>
    );
}
