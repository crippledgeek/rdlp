// List view container for search results. Renders ResultRow components.

import { ResultRow } from "./ResultRow";
import type { DownloadOptions, SearchResultPreview } from "../types";

interface ResultsListProps {
    results: SearchResultPreview[];
    focusIndex: number;
    selectedIndices: Set<number>;
    onDownload: (url: string) => void;
    onDownloadWithOptions: (url: string, options: Partial<DownloadOptions>) => void;
    onOpenFormatDialog: (url: string) => void;
    onToggleSelect: (index: number) => void;
}

/** Scrollable list view rendering ResultRow for each search result. */
export function ResultsList({
    results,
    focusIndex,
    selectedIndices,
    onDownload,
    onDownloadWithOptions,
    onOpenFormatDialog,
    onToggleSelect,
}: ResultsListProps) {
    return (
        <div className="border-t border-border">
            {results.map((result, idx) => (
                <ResultRow
                    key={`${idx}-${result.video_url}`}
                    result={result}
                    focused={focusIndex === idx}
                    selected={selectedIndices.has(idx)}
                    onDownload={onDownload}
                    onDownloadWithOptions={onDownloadWithOptions}
                    onOpenFormatDialog={onOpenFormatDialog}
                    onToggleSelect={() => onToggleSelect(idx)}
                />
            ))}
        </div>
    );
}
