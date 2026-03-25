// FormatsTable: TanStack Table data model with HTML table rendering.
// Visual grouping: Video+Audio / Video Only / Audio Only sections.
// Row selection via uiStore.setSelectedFormat().
// Virtualization via TanStack Virtual for > 50 rows.

import { useRef, useMemo } from "react";
import {
    useReactTable,
    getCoreRowModel,
    getSortedRowModel,
    flexRender,
    type SortingState,
    type Row,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useStore } from "@tanstack/react-store";
import { useState } from "react";
import { ChevronUp, ChevronDown, ChevronsUpDown } from "lucide-react";
import { uiStore, setSelectedFormat } from "@/stores/uiStore";
import type { FormatInfo } from "@/types";
import { formatColumns } from "./formatColumns";
import { FormatGroupHeader } from "./FormatGroupHeader";
import { FormatRow } from "./FormatRow";
import { cn } from "@/lib/utils";

interface FormatsTableProps {
    formats: FormatInfo[];
}

function getBestFormatId(formats: FormatInfo[]): string | null {
    const muxed = formats.filter((f) => f.has_video && f.has_audio);
    if (muxed.length === 0) return null;
    const best = muxed.sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0];
    return best?.format_id ?? null;
}

export function FormatsTable({ formats }: FormatsTableProps) {
    const parentRef = useRef<HTMLDivElement>(null);
    const [sorting, setSorting] = useState<SortingState>([
        { id: "quality", desc: true },
    ]);
    const selectedFormatId = useStore(uiStore, (s) => s.selectedFormatId);

    const bestFormatId = useMemo(() => getBestFormatId(formats), [formats]);

    const table = useReactTable({
        data: formats,
        columns: formatColumns,
        state: { sorting },
        onSortingChange: setSorting,
        getCoreRowModel: getCoreRowModel(),
        getSortedRowModel: getSortedRowModel(),
    });

    const allRows = table.getRowModel().rows;

    // Group rows into three categories
    const { videoAudio, videoOnly, audioOnly } = useMemo(() => {
        const va: Row<FormatInfo>[] = [];
        const vo: Row<FormatInfo>[] = [];
        const ao: Row<FormatInfo>[] = [];
        for (const row of allRows) {
            const f = row.original;
            if (f.has_video && f.has_audio) va.push(row);
            else if (f.has_video) vo.push(row);
            else ao.push(row);
        }
        return { videoAudio: va, videoOnly: vo, audioOnly: ao };
    }, [allRows]);

    // Flat list of all rows with group header markers for virtualization
    type RowItem =
        | { type: "header"; label: string; count: number; key: string }
        | { type: "row"; row: Row<FormatInfo> };

    const items = useMemo((): RowItem[] => {
        const result: RowItem[] = [];
        if (videoAudio.length > 0) {
            result.push({ type: "header", label: "Video + Audio", count: videoAudio.length, key: "va" });
            videoAudio.forEach((row) => result.push({ type: "row", row }));
        }
        if (videoOnly.length > 0) {
            result.push({ type: "header", label: "Video Only", count: videoOnly.length, key: "vo" });
            videoOnly.forEach((row) => result.push({ type: "row", row }));
        }
        if (audioOnly.length > 0) {
            result.push({ type: "header", label: "Audio Only", count: audioOnly.length, key: "ao" });
            audioOnly.forEach((row) => result.push({ type: "row", row }));
        }
        return result;
    }, [videoAudio, videoOnly, audioOnly]);

    const useVirtual = formats.length > 50;

    const virtualizer = useVirtualizer({
        count: items.length,
        getScrollElement: () => parentRef.current,
        estimateSize: (i) => (items[i]?.type === "header" ? 28 : 32),
        overscan: 5,
        enabled: useVirtual,
    });

    return (
        <div ref={parentRef} className="h-full overflow-y-auto">
            <table
                className="w-full border-collapse"
                style={{ tableLayout: "fixed" }}
            >
                <thead className="sticky top-0 z-20 bg-[var(--surface-deepest)]">
                    {table.getHeaderGroups().map((hg) => (
                        <tr key={hg.id} className="border-b border-[#1a1a2e]">
                            {hg.headers.map((header) => (
                                <th
                                    key={header.id}
                                    onClick={header.column.getToggleSortingHandler()}
                                    style={{ width: header.getSize() }}
                                    className={cn(
                                        "table-th text-left select-none",
                                        header.column.getCanSort() && "cursor-pointer hover:text-[#aaaaaa]",
                                        header.column.id === "filesize" && "text-right",
                                    )}
                                >
                                    <span className="flex items-center gap-1">
                                        {flexRender(header.column.columnDef.header, header.getContext())}
                                        {header.column.getCanSort() && (
                                            <span className="opacity-40">
                                                {header.column.getIsSorted() === "asc" ? (
                                                    <ChevronUp className="w-3 h-3" />
                                                ) : header.column.getIsSorted() === "desc" ? (
                                                    <ChevronDown className="w-3 h-3" />
                                                ) : (
                                                    <ChevronsUpDown className="w-3 h-3" />
                                                )}
                                            </span>
                                        )}
                                    </span>
                                </th>
                            ))}
                        </tr>
                    ))}
                </thead>

                <tbody
                    style={
                        useVirtual
                            ? { height: virtualizer.getTotalSize(), position: "relative" }
                            : undefined
                    }
                >
                    {useVirtual
                        ? virtualizer.getVirtualItems().map((virtualItem) => {
                            const item = items[virtualItem.index];
                            if (!item) return null;

                            if (item.type === "header") {
                                return (
                                    <tr
                                        key={item.key}
                                        style={{
                                            position: "absolute",
                                            top: virtualItem.start,
                                            left: 0,
                                            width: "100%",
                                            height: virtualItem.size,
                                        }}
                                    >
                                        <td
                                            colSpan={7}
                                            className="py-1 px-3 bg-[var(--surface-deepest)] border-y border-[#1a1a2e]"
                                        >
                                            <span className="text-[10px] font-bold uppercase tracking-widest text-[#666666]">
                                                {item.label}
                                            </span>
                                            <span className="ml-2 text-[10px] text-[#444444]">
                                                {item.count}
                                            </span>
                                        </td>
                                    </tr>
                                );
                            }

                            return (
                                <tr
                                    key={item.row.id}
                                    style={{
                                        position: "absolute",
                                        top: virtualItem.start,
                                        left: 0,
                                        width: "100%",
                                        height: virtualItem.size,
                                    }}
                                >
                                    <FormatRow
                                        row={item.row}
                                        isSelected={item.row.original.format_id === selectedFormatId}
                                        isBest={item.row.original.format_id === bestFormatId}
                                        onClick={setSelectedFormat}
                                    />
                                </tr>
                            );
                        })
                        : items.map((item) => {
                            if (item.type === "header") {
                                return <FormatGroupHeader key={item.key} label={item.label} count={item.count} />;
                            }
                            return (
                                <FormatRow
                                    key={item.row.id}
                                    row={item.row}
                                    isSelected={item.row.original.format_id === selectedFormatId}
                                    isBest={item.row.original.format_id === bestFormatId}
                                    onClick={setSelectedFormat}
                                />
                            );
                        })}
                </tbody>
            </table>
        </div>
    );
}
