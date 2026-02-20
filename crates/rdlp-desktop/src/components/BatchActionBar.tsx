// Slide-up bar when results are selected, showing batch download actions.

import { Button } from "@/components/ui/button";

interface BatchActionBarProps {
    count: number;
    onDownloadAll: () => void;
    onClearSelection: () => void;
}

/** Batch action bar: "[N] selected -- [Download All] [Clear]". */
export function BatchActionBar({
    count,
    onDownloadAll,
    onClearSelection,
}: BatchActionBarProps) {
    if (count === 0) return null;

    return (
        <div className="flex items-center gap-2.5 h-9 px-3 bg-card border-t border-border shrink-0 animate-in slide-in-from-bottom duration-150">
            <span className="font-mono text-[11px] text-muted-foreground">
                {count} selected
            </span>
            <Button
                variant="outline"
                size="sm"
                className="border-primary text-primary hover:bg-primary/10"
                onClick={onDownloadAll}
            >
                Download All
            </Button>
            <Button
                variant="outline"
                size="sm"
                onClick={onClearSelection}
            >
                Clear
            </Button>
        </div>
    );
}
