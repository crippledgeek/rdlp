// TanStack Table column definitions for the search results table.
//
// Mirrors the formatColumns.tsx pattern: a hook returning stable column
// definitions and the total column size for percentage-based layout.

import { useMemo } from "react";
import { createColumnHelper } from "@tanstack/react-table";
import type { ColumnDef } from "@tanstack/react-table";
import { Checkbox } from "@/components/ui/checkbox";
import { ResultRowActions } from "../ResultRowActions";
import { formatDuration, formatViews } from "./searchUtils";
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
    const columnHelper = useMemo(() => createColumnHelper<SearchResultPreview>(), []);

    const columns = useMemo(() => [
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
                    {row.original.thumbnail_url ? (
                        <img
                            className="w-full h-full object-cover"
                            src={row.original.thumbnail_url}
                            alt=""
                        />
                    ) : (
                        <div className="w-full h-full bg-muted" />
                    )}
                </div>
            ),
        }),
        columnHelper.accessor("title", {
            header: "Title",
            size: 400,
            sortingFn: "text",
            enableResizing: true,
            cell: ({ row }) => (
                <div className="flex flex-col gap-px min-w-0">
                    <div className="text-[13px] font-medium text-foreground truncate" title={row.original.title}>
                        {row.original.title}
                    </div>
                    {row.original.view_count !== null && (
                        <div className="text-[11px] text-muted-foreground">
                            {formatViews(row.original.view_count)}
                        </div>
                    )}
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
    ], [columnHelper, focusIndex, onDownload, onDownloadWithOptions, onOpenFormatDialog]);

    const totalColumnSize = useMemo(
        () => columns.reduce((sum, col) => sum + (col.size ?? 0), 0),
        [columns],
    );

    return { columns, totalColumnSize };
}
