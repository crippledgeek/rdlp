// Action buttons (download, format, options) for a single search result row.

import { useState, useRef, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { useOnClickOutside } from "usehooks-ts";
import { Download, LayoutGrid, ChevronDown } from "lucide-react";
import { settingsQueryOptions } from "../api/settings";
import { FormatOptionsPanel, buildDefaultOptions } from "./FormatOptionsPanel";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { DownloadOptions, SearchResultPreview } from "../types";

interface ResultRowActionsProps {
    result: SearchResultPreview;
    visible: boolean;
    onDownload: (url: string, title: string) => void;
    onDownloadWithOptions: (url: string, title: string, options: Partial<DownloadOptions>) => void;
    onOpenFormatDialog: (url: string) => void;
}

/** Action button group + FormatOptionsPanel popover for a search result row. */
export function ResultRowActions({
    result,
    visible,
    onDownload,
    onDownloadWithOptions,
    onOpenFormatDialog,
}: ResultRowActionsProps) {
    const { data: settings = null } = useQuery(settingsQueryOptions());
    const [showOptions, setShowOptions] = useState(false);
    const [panelOptions, setPanelOptions] = useState<DownloadOptions>(() =>
        buildDefaultOptions(settings),
    );
    const popoverRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!showOptions) {
            setPanelOptions(buildDefaultOptions(settings));
        }
    }, [settings, showOptions]);

    useOnClickOutside(popoverRef, () => setShowOptions(false));

    return (
        <>
            <div className={cn(
                "flex gap-0.5 shrink-0 opacity-0 transition-opacity",
                visible && "opacity-100",
            )}>
                <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 hover:text-primary hover:bg-primary/10"
                    onClick={(e) => { e.stopPropagation(); onDownload(result.video_url, result.title); }}
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
                            onDownloadWithOptions(result.video_url, result.title, panelOptions);
                            setShowOptions(false);
                        }}
                    >
                        Download with Options
                    </Button>
                </div>
            )}
        </>
    );
}
