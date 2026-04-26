// TanStack Table column definitions for search results.
// Sortable by title, duration, views, and age.

import { createColumnHelper } from "@tanstack/react-table";
import type { Row } from "@tanstack/react-table";
import type { SearchResultPreview } from "@/types";
import { Thumbnail } from "@/components/Thumbnail";

const columnHelper = createColumnHelper<SearchResultPreview>();

function formatDuration(seconds: number | null): string {
    if (seconds === null || seconds === undefined) return "—";
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
}

function formatViews(count: number | null): string {
    if (count === null || count === undefined) return "—";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`;
    return String(count);
}

function formatDate(dateStr: string | null): string {
    if (!dateStr) return "—";
    // Handle YYYYMMDD format
    if (/^\d{8}$/.test(dateStr)) {
        const y = dateStr.slice(0, 4);
        const m = dateStr.slice(4, 6);
        const d = dateStr.slice(6, 8);
        return `${y}-${m}-${d}`;
    }
    return dateStr;
}

const ageSortFn = (rowA: Row<SearchResultPreview>, rowB: Row<SearchResultPreview>) => {
    const a = rowA.original.upload_date;
    const b = rowB.original.upload_date;
    if (a === null && b === null) return 0;
    if (a === null) return 1;
    if (b === null) return -1;
    return a.localeCompare(b);
};

export const searchResultColumns = [
    columnHelper.display({
        id: "thumbnail",
        header: "",
        size: 72,
        enableSorting: false,
        cell: ({ row }) => {
            const r = row.original;
            return (
                <div className="w-16 h-9 rounded-[3px] overflow-hidden bg-[var(--surface-elevated)] relative">
                    <Thumbnail
                        src={r.thumbnail_url}
                        alt=""
                        className="w-full h-full object-cover"
                    />
                    {r.duration != null && (
                        <span className="absolute bottom-0.5 right-0.5 bg-black/80 text-white text-[9px] px-1 rounded-[2px] font-mono">
                            {Math.floor(r.duration / 60)}:{String(r.duration % 60).padStart(2, "0")}
                        </span>
                    )}
                </div>
            );
        },
    }),
    columnHelper.accessor("title", {
        header: "Title",
        // Smaller fixed allocation so the row total (72 thumb + 280 + 140
        // + 80 + 80 = 652) fits inside narrow content panes without the
        // rightmost numeric columns getting clipped. Title text truncates
        // via the `truncate` class on each `<td>`; the thumbnail beside
        // each row provides the visual cue.
        size: 280,
        sortingFn: "text",
    }),
    columnHelper.accessor("uploader", {
        header: "Uploader",
        size: 140,
        sortingFn: "text",
        sortUndefined: "last",
        cell: ({ getValue }) => getValue() ?? "—",
    }),
    columnHelper.accessor("duration", {
        header: "Duration",
        size: 80,
        sortDescFirst: true,
        sortUndefined: "last",
        sortingFn: "basic",
        cell: ({ getValue }) => formatDuration(getValue()),
    }),
    columnHelper.accessor("view_count", {
        id: "views",
        header: "Views",
        size: 80,
        sortDescFirst: true,
        sortUndefined: "last",
        sortingFn: "basic",
        cell: ({ getValue }) => formatViews(getValue()),
    }),
    columnHelper.accessor("upload_date", {
        id: "age",
        header: "Date",
        size: 90,
        sortingFn: ageSortFn,
        cell: ({ getValue }) => formatDate(getValue()),
    }),
];
