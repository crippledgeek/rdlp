// Search results table with TanStack Table: column visibility toggle, sortable
// headers with resize handles, checkbox selection, and result count footer.

import { useEffect, useMemo, useRef } from "react";
import type { Table as TanStackTable, ColumnDef } from "@tanstack/react-table";
import { flexRender } from "@tanstack/react-table";
import { ChevronUp, ChevronDown, Columns3 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
    DropdownMenu,
    DropdownMenuCheckboxItem,
    DropdownMenuContent,
    DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
    Table,
    TableBody,
    TableCell,
    TableFooter,
    TableHead,
    TableHeader,
    TableRow,
} from "@/components/ui/table";
import type { SearchResultPreview } from "../types";

interface ResultsTableSectionProps {
    table: TanStackTable<SearchResultPreview>;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    columns: ColumnDef<SearchResultPreview, any>[];
    totalColumnSize: number;
    focusIndex: number;
    resultCount: number;
}

/** Non-hideable column IDs (always visible). */
const FIXED_COLUMNS = new Set(["select", "thumbnail", "actions"]);

/** Search results table with sortable headers, resize handles, and column visibility. */
export function ResultsTableSection({
    table,
    columns,
    totalColumnSize,
    focusIndex,
    resultCount,
}: ResultsTableSectionProps) {
    const rowRefs = useRef<Map<number, HTMLTableRowElement>>(new Map());

    // Compute total size from visible columns only (so percentages stay correct when columns hide)
    const visibleTotalSize = useMemo(
        () => table.getVisibleFlatColumns().reduce((sum, col) => sum + col.getSize(), 0),
        // eslint-disable-next-line react-hooks/exhaustive-deps
        [table, totalColumnSize, table.getState().columnVisibility],
    );

    // Scroll focused row into view
    useEffect(() => {
        if (focusIndex >= 0) {
            const el = rowRefs.current.get(focusIndex);
            el?.scrollIntoView({ block: "nearest" });
        }
    }, [focusIndex]);

    const toggleableColumns = table
        .getAllColumns()
        .filter((col) => col.getCanHide() && !FIXED_COLUMNS.has(col.id));

    return (
        <div className="flex flex-col flex-1 min-h-0">
            {/* Column visibility dropdown */}
            {toggleableColumns.length > 0 && (
                <div className="flex justify-end px-3 py-1 shrink-0">
                    <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                            <Button variant="ghost" size="sm" className="h-7 text-xs text-muted-foreground gap-1">
                                <Columns3 className="h-3.5 w-3.5" />
                                Columns
                            </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                            {toggleableColumns.map((col) => (
                                <DropdownMenuCheckboxItem
                                    key={col.id}
                                    checked={col.getIsVisible()}
                                    onCheckedChange={(checked) => col.toggleVisibility(checked)}
                                    className="capitalize text-xs"
                                >
                                    {typeof col.columnDef.header === "string"
                                        ? col.columnDef.header
                                        : col.id}
                                </DropdownMenuCheckboxItem>
                            ))}
                        </DropdownMenuContent>
                    </DropdownMenu>
                </div>
            )}

            {/* Scrollable table */}
            <ScrollArea className="flex-1 min-h-0" type="auto">
                <Table className="text-xs" style={{ tableLayout: "fixed" }}>
                    <TableHeader className="sticky top-0 z-10">
                        {table.getHeaderGroups().map((headerGroup) => (
                            <TableRow
                                key={headerGroup.id}
                                className="bg-foreground/[0.06] backdrop-blur-sm hover:bg-foreground/[0.06]"
                            >
                                {headerGroup.headers.map((header) => {
                                    const align = (header.column.columnDef.meta as { align?: string } | undefined)?.align;
                                    const isSorted = header.column.getIsSorted();
                                    return (
                                        <TableHead
                                            key={header.id}
                                            style={{ width: `${(header.getSize() / visibleTotalSize * 100).toFixed(1)}%` }}
                                            className={cn(
                                                "h-8 px-3 text-[10px] font-bold tracking-wider uppercase select-none transition-colors relative",
                                                header.column.getCanSort() && "cursor-pointer hover:text-foreground",
                                                align === "right" && "text-right",
                                                isSorted ? "text-foreground" : "text-muted-foreground",
                                            )}
                                            onClick={header.column.getToggleSortingHandler()}
                                        >
                                            <span className="inline-flex items-center gap-1">
                                                {flexRender(header.column.columnDef.header, header.getContext())}
                                                {isSorted === "asc" && <ChevronUp className="size-3" />}
                                                {isSorted === "desc" && <ChevronDown className="size-3" />}
                                            </span>
                                            {/* Resize handle */}
                                            {header.column.getCanResize() && (
                                                <div
                                                    onMouseDown={header.getResizeHandler()}
                                                    onTouchStart={header.getResizeHandler()}
                                                    className={cn(
                                                        "absolute right-0 top-0 h-full w-1 cursor-col-resize select-none touch-none",
                                                        "hover:bg-primary/40",
                                                        header.column.getIsResizing() && "bg-primary/60",
                                                    )}
                                                />
                                            )}
                                        </TableHead>
                                    );
                                })}
                            </TableRow>
                        ))}
                    </TableHeader>
                    <TableBody>
                        {table.getRowModel().rows.length > 0 ? (
                            table.getRowModel().rows.map((row) => (
                                <TableRow
                                    key={row.id}
                                    ref={(el) => {
                                        if (el) rowRefs.current.set(row.index, el);
                                        else rowRefs.current.delete(row.index);
                                    }}
                                    className={cn(
                                        "group h-[52px] border-b border-white/[0.03] border-l-2 border-l-transparent transition-colors",
                                        "hover:bg-card",
                                        row.index === focusIndex && "bg-card border-l-primary",
                                        row.getIsSelected() && "bg-primary/15 border-l-primary",
                                    )}
                                    onClick={() => row.toggleSelected()}
                                    data-focused={row.index === focusIndex || undefined}
                                >
                                    {row.getVisibleCells().map((cell) => {
                                        const align = (cell.column.columnDef.meta as { align?: string } | undefined)?.align;
                                        return (
                                            <TableCell
                                                key={cell.id}
                                                className={cn(
                                                    "px-3 py-1 relative",
                                                    align === "right" && "text-right",
                                                )}
                                            >
                                                {flexRender(cell.column.columnDef.cell, cell.getContext())}
                                            </TableCell>
                                        );
                                    })}
                                </TableRow>
                            ))
                        ) : (
                            <TableRow className="hover:bg-transparent">
                                <TableCell colSpan={columns.length} className="text-center py-8 text-muted-foreground">
                                    No results.
                                </TableCell>
                            </TableRow>
                        )}
                    </TableBody>
                    <TableFooter className="sticky bottom-0 z-10 bg-card/95 backdrop-blur-sm">
                        <TableRow className="hover:bg-transparent">
                            <TableCell colSpan={columns.length} className="px-3 py-1.5 text-[10px] text-muted-foreground">
                                {resultCount} result{resultCount !== 1 ? "s" : ""}
                            </TableCell>
                        </TableRow>
                    </TableFooter>
                </Table>
            </ScrollArea>
        </div>
    );
}
