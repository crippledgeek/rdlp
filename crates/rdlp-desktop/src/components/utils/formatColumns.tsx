// TanStack Table column definitions for the format selection table.
//
// Extracted as a hook because it uses useMemo for stable column references.

import { useMemo } from "react";
import { createColumnHelper } from "@tanstack/react-table";
import type { ColumnDef } from "@tanstack/react-table";
import { cn } from "@/lib/utils";
import type { FormatInfo } from "../../types";
import { formatBitrate, formatResolution, formatSize, formatType } from "./formatUtils";

interface FormatColumnsResult {
    // TanStack columnHelper produces mixed accessor value types (number | undefined,
    // string, etc.) that cannot be represented as ColumnDef<FormatInfo, unknown>[].
    // Using `any` for the value parameter matches TanStack's own internal usage.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    columns: ColumnDef<FormatInfo, any>[];
    totalColumnSize: number;
}

/** Stable column definitions and total width for the format table. */
export function useFormatColumns(): FormatColumnsResult {
    const columnHelper = useMemo(() => createColumnHelper<FormatInfo>(), []);

    const columns = useMemo(() => [
        columnHelper.accessor((row) => row.height ?? undefined, {
            id: "height",
            header: "Resolution",
            size: 170,
            sortDescFirst: true,
            sortUndefined: "last",
            sortingFn: "basic",
            cell: ({ row }) => (
                <span className="font-mono">{formatResolution(row.original)}</span>
            ),
            meta: { align: "left" as const },
        }),
        columnHelper.accessor("ext", {
            header: "Ext",
            size: 90,
            sortingFn: "text",
            meta: { align: "left" as const },
        }),
        columnHelper.accessor((row) => row.fps ?? undefined, {
            id: "fps",
            header: "FPS",
            size: 90,
            sortDescFirst: true,
            sortUndefined: "last",
            sortingFn: "basic",
            cell: ({ getValue }) => {
                const fps = getValue();
                return fps !== undefined ? Math.round(fps) : "\u2014";
            },
            meta: { align: "right" as const },
        }),
        columnHelper.accessor((row) => row.tbr ?? undefined, {
            id: "tbr",
            header: "Bitrate",
            size: 130,
            sortDescFirst: true,
            sortUndefined: "last",
            sortingFn: "basic",
            cell: ({ row }) => formatBitrate(row.original.tbr),
            meta: { align: "right" as const },
        }),
        columnHelper.accessor((row) => row.vcodec ?? undefined, {
            id: "vcodec",
            header: "Video",
            size: 120,
            sortUndefined: "last",
            sortingFn: "text",
            cell: ({ getValue }) => getValue() ?? "\u2014",
            meta: { align: "left" as const },
        }),
        columnHelper.accessor((row) => row.acodec ?? undefined, {
            id: "acodec",
            header: "Audio",
            size: 110,
            sortUndefined: "last",
            sortingFn: "text",
            cell: ({ getValue }) => getValue() ?? "\u2014",
            meta: { align: "left" as const },
        }),
        columnHelper.accessor((row) => row.filesize ?? undefined, {
            id: "filesize",
            header: "Size",
            size: 130,
            sortDescFirst: true,
            sortUndefined: "last",
            sortingFn: "basic",
            cell: ({ row }) => formatSize(row.original.filesize),
            meta: { align: "right" as const },
        }),
        columnHelper.display({
            id: "type",
            header: "Type",
            size: 160,
            enableSorting: false,
            cell: ({ row }) => {
                const type = formatType(row.original);
                return (
                    <span className={cn(
                        "inline-block px-1.5 py-0.5 rounded text-[9px] font-bold tracking-wider uppercase",
                        type.className,
                    )}>
                        {type.label}
                    </span>
                );
            },
            meta: { align: "center" as const },
        }),
    ], [columnHelper]);

    const totalColumnSize = useMemo(
        () => columns.reduce((sum, col) => sum + (col.size ?? 0), 0),
        [columns],
    );

    return { columns, totalColumnSize };
}
