// TanStack Table column definitions for the search results table.
//
// Mirrors the formatColumns.tsx pattern: a hook returning stable column
// definitions and the total column size for percentage-based layout.

import { createColumnHelper } from "@tanstack/react-table";
import type { ColumnDef, Row } from "@tanstack/react-table";
import { Checkbox } from "@/components/ui/checkbox";
import { parseUploadTimestamp } from "@/lib/utils";
import { Thumbnail } from "../Thumbnail";
import { ResultRowActions } from "../ResultRowActions";
import { formatDuration, formatCompactDate, formatViews } from "./searchUtils";
import type { DownloadOptions, SearchResultPreview } from "../../types";

interface SearchColumnCallbacks {
    onDownload: (url: string, title: string) => void;
    onDownloadWithOptions: (url: string, title: string, options: Partial<DownloadOptions>) => void;
    onOpenFormatDialog: (url: string) => void;
    focusIndex: number;
}

interface SearchColumnsResult {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    columns: ColumnDef<SearchResultPreview, any>[];
    totalColumnSize: number;
}

/** Stable column definitions for the search results table. */
export function useSearchColumns(callbacks: SearchColumnCallbacks): SearchColumnsResult {
    const { onDownload, onDownloadWithOptions, onOpenFormatDialog, focusIndex } = callbacks;
    const columnHelper = createColumnHelper<SearchResultPreview>();

    const ageSortFn = (rowA: Row<SearchResultPreview>, rowB: Row<SearchResultPreview>) => {
        const a = rowA.original.upload_date;
        const b = rowB.original.upload_date;
        if (a === null && b === null) return 0;
        if (a === null) return 1;
        if (b === null) return -1;
        const dateA = parseUploadTimestamp(a);
        const dateB = parseUploadTimestamp(b);
        if (dateA === null && dateB === null) return 0;
        if (dateA === null) return 1;
        if (dateB === null) return -1;
        return dateA.getTime() - dateB.getTime();
    };

    const columns: ColumnDef<SearchResultPreview, any>[] = [ // eslint-disable-line @typescript-eslint/no-explicit-any
        columnHelper.display({
            id: "select",
            size: 40,
            enableSorting: false,
            enableResizing: false,
            header: ({ table }) => (
                <Checkbox
                    checked={table.getIsAllRowsSelected()}
                    onCheckedChange={(checked) => table.toggleAllRowsSelected(checked === true)}
                    aria-label="Select all"
                    className="ml-1"
                />
            ),
            cell: ({ row }) => (
                <Checkbox
                    checked={row.getIsSelected()}
                    onCheckedChange={(checked) => row.toggleSelected(checked === true)}
                    aria-label="Select row"
                    className="ml-1"
                    onClick={(e) => e.stopPropagation()}
                />
            ),
        }),
        columnHelper.display({
            id: "thumbnail",
            header: "",
            size: 72,
            enableSorting: false,
            enableResizing: false,
            cell: ({ row }) => (
                <div className="w-16 h-9 shrink-0 rounded-sm overflow-hidden bg-background">
                    <Thumbnail
                        src={row.original.thumbnail_url}
                        alt=""
                        className="w-full h-full object-cover"
                    />
                </div>
            ),
        }),
        columnHelper.accessor("title", {
            header: "Title",
            size: 400,
            sortingFn: "text",
            enableResizing: true,
            cell: ({ row }) => (
                <div className="text-[13px] font-medium text-foreground truncate" title={row.original.title}>
                    {row.original.title}
                </div>
            ),
        }),
        columnHelper.accessor("duration", {
            header: "Duration",
            size: 80,
            sortDescFirst: true,
            sortUndefined: "last",
            sortingFn: "basic",
            enableResizing: true,
            cell: ({ row }) => (
                <span className="font-mono text-xs text-muted-foreground">
                    {formatDuration(row.original.duration)}
                </span>
            ),
            meta: { align: "right" as const },
        }),
        columnHelper.accessor("view_count", {
            id: "views",
            header: "Views",
            size: 90,
            sortDescFirst: true,
            sortUndefined: "last",
            sortingFn: "basic",
            enableResizing: true,
            cell: ({ row }) => (
                <span className="font-mono text-xs text-muted-foreground">
                    {formatViews(row.original.view_count)}
                </span>
            ),
            meta: { align: "right" as const },
        }),
        columnHelper.accessor("upload_date", {
            id: "age",
            header: "Age",
            size: 80,
            sortingFn: ageSortFn,
            enableResizing: true,
            cell: ({ row }) => (
                <span className="font-mono text-xs text-muted-foreground">
                    {formatCompactDate(row.original.upload_date)}
                </span>
            ),
            meta: { align: "right" as const },
        }),
        columnHelper.display({
            id: "actions",
            header: "",
            size: 100,
            enableSorting: false,
            enableResizing: false,
            cell: ({ row }) => (
                <ResultRowActions
                    result={row.original}
                    visible={row.getIsSelected() || row.index === focusIndex}
                    onDownload={onDownload}
                    onDownloadWithOptions={onDownloadWithOptions}
                    onOpenFormatDialog={onOpenFormatDialog}
                />
            ),
        }),
    ];

    const totalColumnSize = columns.reduce((sum, col) => sum + (col.size ?? 0), 0);

    return { columns, totalColumnSize };
}
