import { flexRender } from "@tanstack/react-table";
import type { Row } from "@tanstack/react-table";
import { cn } from "@/lib/utils";
import {
    TableCell,
    TableRow,
} from "@/components/ui/table";
import type { FormatInfo } from "../types";

// -- Types ----------------------------------------------------------------

export interface FormatGroup {
    label: string;
    rows: Row<FormatInfo>[];
}

interface FormatGroupSectionProps {
    group: FormatGroup;
    columnCount: number;
    selectedId: string | null;
    mergeId: string | null;
    exprMatches: Set<string>;
    onRowClick: (id: string, e: React.MouseEvent) => void;
}

/** Renders a section header + rows for a format group (Video+Audio, Video Only, Audio Only). */
export function FormatGroupSection({ group, columnCount, selectedId, mergeId, exprMatches, onRowClick }: FormatGroupSectionProps) {
    return (
        <>
            {/* Section header */}
            <TableRow className="hover:bg-transparent bg-transparent border-none">
                <TableCell
                    colSpan={columnCount}
                    className="px-3 py-1.5 text-[10px] font-bold uppercase tracking-widest text-foreground/40"
                >
                    {group.label}
                </TableCell>
            </TableRow>

            {/* Format rows */}
            {group.rows.map((row, i) => {
                const f = row.original;
                const isSelected = f.format_id === selectedId;
                const isMerge = f.format_id === mergeId;
                const isExprMatch = exprMatches.has(f.format_id);
                const isEven = i % 2 === 0;

                const cellColorClass = isSelected || isMerge
                    ? "text-foreground"
                    : isExprMatch
                        ? "text-yellow-500"
                        : "text-foreground/60";

                return (
                    <TableRow
                        key={row.id}
                        className={cn(
                            "cursor-pointer transition-colors border-b-foreground/[0.06]",
                            // Base: zebra stripe + hover (only for unselected rows)
                            !isSelected && !isMerge && (isEven ? "bg-transparent" : "bg-foreground/[0.02]"),
                            !isSelected && !isMerge && "hover:bg-foreground/[0.06]",
                            // Selected: strong primary highlight, no hover override
                            isSelected && "bg-primary/30 border-l-[3px] border-l-primary hover:bg-primary/35",
                            // Merge: distinct secondary highlight
                            isMerge && "bg-foreground/15 border-l-[3px] border-l-foreground/40 hover:bg-foreground/18",
                            isExprMatch && !isSelected && !isMerge && "bg-yellow-500/5",
                        )}
                        onClick={(e) => onRowClick(f.format_id, e)}
                    >
                        {row.getVisibleCells().map((cell) => {
                            const align = (cell.column.columnDef.meta as { align?: string } | undefined)?.align;
                            return (
                                <TableCell
                                    key={cell.id}
                                    className={cn(
                                        "px-3",
                                        cellColorClass,
                                        align === "right" && "text-right",
                                        align === "center" && "text-center",
                                    )}
                                >
                                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                                </TableCell>
                            );
                        })}
                    </TableRow>
                );
            })}
        </>
    );
}
