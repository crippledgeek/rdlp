import { useEffect, useRef, useState } from "react";
import { formatDistanceToNow } from "date-fns";
import { useQuery } from "@tanstack/react-query";
import { Download, ChevronDown } from "lucide-react";
import { settingsQueryOptions } from "../api/settings";
import { FormatOptionsPanel, buildDefaultOptions } from "./FormatOptionsPanel";
import { Button } from "@/components/ui/button";
import { cn, parseUploadTimestamp } from "@/lib/utils";
import type {
    DownloadOptions,
    SearchResultPreview,
} from "../types";

interface ResultCardProps {
    result: SearchResultPreview;
    onDownload: (url: string) => void;
    onDownloadWithOptions: (url: string, options: Partial<DownloadOptions>) => void;
    onOpenFormatDialog: (url: string) => void;
}

/** Format seconds into "M:SS" display string. */
function formatDuration(seconds: number | null): string {
    if (seconds === null) return "";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Format an upload timestamp string into a relative date (e.g. "3 days ago"). */
function formatUploadDate(timestamp: string | null): string {
    if (timestamp === null) return "";
    const date = parseUploadTimestamp(timestamp);
    if (date === null) return "";
    return formatDistanceToNow(date, { addSuffix: true });
}

/** Format a view count into a human-readable string (e.g. "1.2K views"). */
function formatViews(count: number | null): string {
    if (count === null) return "";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K views`;
    return `${count} views`;
}


/** Displays a single search result as a card with thumbnail, title, and metadata. */
export function ResultCard({
    result,
    onDownload,
    onDownloadWithOptions,
    onOpenFormatDialog,
}: ResultCardProps) {
    const duration = formatDuration(result.duration);
    const views = formatViews(result.view_count);
    const uploadDate = formatUploadDate(result.upload_date);
    const { data: settings = null } = useQuery(settingsQueryOptions());

    const [showOptions, setShowOptions] = useState(false);
    const [panelOptions, setPanelOptions] = useState<DownloadOptions>(() =>
        buildDefaultOptions(settings),
    );
    const popoverRef = useRef<HTMLDivElement>(null);

    // Update panel defaults when settings change
    useEffect(() => {
        if (!showOptions) {
            setPanelOptions(buildDefaultOptions(settings));
        }
    }, [settings, showOptions]);

    // Dismiss popover on outside click
    useEffect(() => {
        if (!showOptions) return;
        const handler = (e: MouseEvent) => {
            if (
                popoverRef.current &&
                !popoverRef.current.contains(e.target as Node)
            ) {
                setShowOptions(false);
            }
        };
        document.addEventListener("mousedown", handler);
        return () => document.removeEventListener("mousedown", handler);
    }, [showOptions]);

    return (
        <div className={cn(
            "group bg-card border border-border rounded-lg overflow-hidden transition-all duration-250",
            "hover:border-white/[0.08] hover:-translate-y-0.5 hover:shadow-[0_8px_24px_rgba(0,0,0,0.3)]",
        )}>
            <div className="relative aspect-video bg-background overflow-hidden">
                {result.thumbnail_url ? (
                    <img
                        className="w-full h-full object-cover transition-transform duration-500 group-hover:scale-[1.04]"
                        src={result.thumbnail_url}
                        alt={result.title}
                        loading="lazy"
                    />
                ) : (
                    <div className="flex items-center justify-center h-full text-muted-foreground text-xs uppercase tracking-wider">
                        No thumbnail
                    </div>
                )}
                {duration && (
                    <span className="absolute bottom-1.5 left-1.5 px-[7px] py-0.5 bg-black/85 text-white rounded-sm font-mono text-[11px] font-bold tracking-wide backdrop-blur-sm">
                        {duration}
                    </span>
                )}
                <div className="absolute inset-0 bg-gradient-to-t from-black/70 via-black/20 to-transparent flex items-end justify-end p-2.5 opacity-0 transition-opacity group-hover:opacity-100">
                    <div className="flex flex-col items-end gap-1.5">
                        <div className="flex gap-0.5">
                            <Button
                                size="sm"
                                className="bg-primary/10 text-primary hover:bg-primary hover:text-background"
                                onClick={(e) => {
                                    e.stopPropagation();
                                    onDownload(result.video_url);
                                }}
                            >
                                <Download className="h-3.5 w-3.5" />
                                Download
                            </Button>
                            <Button
                                size="icon"
                                className="h-8 w-6 bg-primary/10 text-primary hover:bg-primary hover:text-background"
                                onClick={(e) => {
                                    e.stopPropagation();
                                    setShowOptions((v) => !v);
                                }}
                                aria-label="Download options"
                            >
                                <ChevronDown className="h-3.5 w-3.5" />
                            </Button>
                        </div>
                        <button
                            className="text-[11px] text-white/55 hover:text-foreground cursor-pointer underline-offset-2 hover:underline bg-transparent border-none font-inherit p-0"
                            onClick={(e) => {
                                e.stopPropagation();
                                onOpenFormatDialog(result.video_url);
                            }}
                        >
                            Choose Format
                        </button>
                    </div>

                    {showOptions && (
                        <div
                            className="absolute bottom-full right-0 mb-1.5 w-[280px] bg-muted border border-white/[0.08] rounded-lg shadow-[0_16px_48px_rgba(0,0,0,0.5)] p-2.5 z-50 animate-in fade-in-0 zoom-in-95"
                            ref={popoverRef}
                            onClick={(e) => e.stopPropagation()}
                        >
                            <FormatOptionsPanel
                                value={panelOptions}
                                onChange={setPanelOptions}
                            />
                            <Button
                                className="w-full mt-2.5 bg-primary text-primary-foreground hover:bg-primary/90"
                                onClick={() => {
                                    onDownloadWithOptions(
                                        result.video_url,
                                        panelOptions,
                                    );
                                    setShowOptions(false);
                                }}
                            >
                                Download with Options
                            </Button>
                        </div>
                    )}
                </div>
            </div>

            <div className="px-3 py-2.5">
                <h3 className="mb-1.5 text-[13px] font-semibold leading-snug text-foreground line-clamp-2">{result.title}</h3>
                <div className="flex items-center gap-1.5 text-xs text-muted-foreground mb-2.5">
                    {views && <span>{views}</span>}
                    {views && uploadDate && (
                        <span className="text-[8px] opacity-40">&middot;</span>
                    )}
                    {uploadDate && (
                        <span>{uploadDate}</span>
                    )}
                </div>
            </div>
        </div>
    );
}
