// Slide-up bar when results are selected, showing batch download actions.

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
        <div className="batch-bar">
            <span className="batch-bar-count">{count} selected</span>
            <button className="batch-bar-action batch-bar-primary" onClick={onDownloadAll}>
                Download All
            </button>
            <button className="batch-bar-action" onClick={onClearSelection}>
                Clear
            </button>
        </div>
    );
}
