// Single row in the dense list view. Shows thumbnail, title, metadata columns, and actions.

import { useState, useRef, useEffect } from "react";
import { formatDistanceToNowStrict } from "date-fns";
import { useQuery } from "@tanstack/react-query";
import { Download, LayoutGrid, ChevronDown } from "lucide-react";
import { settingsQueryOptions } from "../api/settings";
import { FormatOptionsPanel, buildDefaultOptions } from "./FormatOptionsPanel";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { cn, parseUploadTimestamp } from "@/lib/utils";
import type {
    DownloadOptions,
    SearchResultPreview,
} from "../types";

interface ResultRowProps {
    result: SearchResultPreview;
    focused: boolean;
    selected: boolean;
    onDownload: (url: string) => void;
    onDownloadWithOptions: (url: string, options: Partial<DownloadOptions>) => void;
    onOpenFormatDialog: (url: string) => void;
    onToggleSelect: () => void;
}

function formatDuration(seconds: number | null): string {
    if (seconds === null) return "\u2014";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatCompactDate(timestamp: string | null): string {
    if (timestamp === null) return "";
    const date = parseUploadTimestamp(timestamp);
    if (date === null) return "";
    try {
        return formatDistanceToNowStrict(date, { addSuffix: false })
            .replace(" seconds", "s").replace(" second", "s")
            .replace(" minutes", "m").replace(" minute", "m")
            .replace(" hours", "h").replace(" hour", "h")
            .replace(" days", "d").replace(" day", "d")
            .replace(" weeks", "w").replace(" week", "w")
            .replace(" months", "mo").replace(" month", "mo")
            .replace(" years", "y").replace(" year", "y");
    } catch {
        return "";
    }
}

function formatViews(count: number | null): string {
    if (count === null) return "";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K views`;
    return `${count} views`;
}


/** Dense list row for a single search result. */
export function ResultRow({
    result,
    focused,
    selected,
    onDownload,
    onDownloadWithOptions,
    onOpenFormatDialog,
    onToggleSelect,
}: ResultRowProps) {
    const { data: settings = null } = useQuery(settingsQueryOptions());
    const [showOptions, setShowOptions] = useState(false);
    const [panelOptions, setPanelOptions] = useState<DownloadOptions>(() =>
        buildDefaultOptions(settings),
    );
    const popoverRef = useRef<HTMLDivElement>(null);
    const rowRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!showOptions) {
            setPanelOptions(buildDefaultOptions(settings));
        }
    }, [settings, showOptions]);

    useEffect(() => {
        if (!showOptions) return;
        const handler = (e: MouseEvent) => {
            if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
                setShowOptions(false);
            }
        };
        document.addEventListener("mousedown", handler);
        return () => document.removeEventListener("mousedown", handler);
    }, [showOptions]);

    // Scroll into view when focused via keyboard
    useEffect(() => {
        if (focused && rowRef.current) {
            rowRef.current.scrollIntoView({ block: "nearest" });
        }
    }, [focused]);

    const duration = formatDuration(result.duration);
    const views = formatViews(result.view_count);
    const date = formatCompactDate(result.upload_date);

    return (
        <div
            ref={rowRef}
            className={cn(
                "group flex items-center gap-2.5 h-[52px] px-3 border-b border-white/[0.03] border-l-2 border-l-transparent relative transition-colors",
                "hover:bg-card",
                focused && "bg-card border-l-primary",
                selected && "bg-primary/10",
            )}
            data-focused={focused || undefined}
        >
            {selected && (
                <Checkbox
                    checked={true}
                    className="shrink-0"
                    onClick={onToggleSelect}
                />
            )}

            <div className="w-16 h-9 shrink-0 rounded-sm overflow-hidden bg-background">
                {result.thumbnail_url ? (
                    <img className="w-full h-full object-cover" src={result.thumbnail_url} alt="" loading="lazy" />
                ) : (
                    <div className="w-full h-full bg-muted" />
                )}
            </div>

            <div className="flex-1 min-w-0 flex flex-col gap-px">
                <div className="text-[13px] font-medium text-foreground truncate" title={result.title}>
                    {result.title}
                </div>
                {views && (
                    <div className="text-[11px] text-muted-foreground">{views}</div>
                )}
            </div>

            <div className="shrink-0 w-12 text-xs font-mono text-muted-foreground text-right">{duration}</div>
            <div className="shrink-0 w-10 text-xs font-mono text-muted-foreground text-right">{date}</div>

            <div className={cn(
                "flex gap-0.5 shrink-0 opacity-0 transition-opacity group-hover:opacity-100",
                focused && "opacity-100",
            )}>
                <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 hover:text-primary hover:bg-primary/10"
                    onClick={(e) => { e.stopPropagation(); onDownload(result.video_url); }}
                    title="Download"
                    aria-label="Download"
                >
                    <Download className="h-3.5 w-3.5" />
                </Button>
                <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 hover:text-primary hover:bg-primary/10"
                    onClick={(e) => { e.stopPropagation(); onOpenFormatDialog(result.video_url); }}
                    title="Choose format"
                    aria-label="Choose format"
                >
                    <LayoutGrid className="h-3.5 w-3.5" />
                </Button>
                <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 hover:text-primary hover:bg-primary/10"
                    onClick={(e) => { e.stopPropagation(); setShowOptions((v) => !v); }}
                    title="Download options"
                    aria-label="Download options"
                >
                    <ChevronDown className="h-3.5 w-3.5" />
                </Button>
            </div>

            {showOptions && (
                <div
                    className="absolute top-full right-3 z-[100] bg-muted border border-white/[0.08] rounded-md p-2.5 shadow-lg min-w-[260px] animate-in fade-in-0 zoom-in-95"
                    ref={popoverRef}
                    onClick={(e) => e.stopPropagation()}
                >
                    <FormatOptionsPanel value={panelOptions} onChange={setPanelOptions} />
                    <Button
                        className="w-full mt-2.5 bg-primary text-primary-foreground hover:bg-primary/90"
                        onClick={() => {
                            onDownloadWithOptions(result.video_url, panelOptions);
                            setShowOptions(false);
                        }}
                    >
                        Download with Options
                    </Button>
                </div>
            )}
        </div>
    );
}
