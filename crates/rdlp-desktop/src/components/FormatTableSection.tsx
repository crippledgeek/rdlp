// Hero format table with preset controls, scrollable table, selection info bar,
// and expert mode expression input.

import type { Table as TanStackTable, ColumnDef } from "@tanstack/react-table";
import { flexRender } from "@tanstack/react-table";
import { ChevronUp, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import {
    ToggleGroup,
    ToggleGroupItem,
} from "@/components/ui/toggle-group";
import { PRESETS } from "./FormatOptionsPanel";
import { FormatGroupSection, type FormatGroup } from "./FormatGroupSection";
import { formatBrief } from "./utils/formatUtils";
import type { FormatInfo } from "../types";

// -- Types ----------------------------------------------------------------

interface FormatTableSectionProps {
    table: TanStackTable<FormatInfo>;
    groups: FormatGroup[];
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    columns: ColumnDef<FormatInfo, any>[];
    totalColumnSize: number;
    selectedId: string | null;
    mergeId: string | null;
    exprMatches: Set<string>;
    presetId: string | null;
    filteredCount: number;
    expertMode: boolean;
    expr: string;
    exprError: string | null;
    findFormat: (id: string) => FormatInfo | undefined;
    onPresetSelect: (id: string) => void;
    onRowClick: (id: string, e: React.MouseEvent) => void;
    onClearSelection: () => void;
    onExpertModeChange: (checked: boolean) => void;
    onExprChange: (value: string) => void;
}

/** Zone 2: Preset toggle group, scrollable format table, selection bar, expert mode. */
export function FormatTableSection({
    table,
    groups,
    columns,
    totalColumnSize,
    selectedId,
    mergeId,
    exprMatches,
    presetId,
    filteredCount,
    expertMode,
    expr,
    exprError,
    findFormat,
    onPresetSelect,
    onRowClick,
    onClearSelection,
    onExpertModeChange,
    onExprChange,
}: FormatTableSectionProps) {
    return (
        <>
            {/* Preset segmented control */}
            <div className="px-5 pt-3 pb-2 shrink-0">
                <ToggleGroup
                    type="single"
                    value={presetId ?? ""}
                    onValueChange={onPresetSelect}
                    className="justify-start gap-0 bg-muted rounded-md p-0.5"
                >
                    {PRESETS.map((p) => (
                        <ToggleGroupItem
                            key={p.id}
                            value={p.id}
                            className="px-3 h-7 text-xs rounded-[5px] text-muted-foreground hover:text-foreground hover:bg-foreground/[0.04] data-[state=on]:bg-foreground/10 data-[state=on]:text-foreground data-[state=on]:shadow-sm"
                        >
                            {p.label}
                        </ToggleGroupItem>
                    ))}
                </ToggleGroup>
            </div>

            {/* Scrollable table */}
            <ScrollArea className="flex-1 min-h-0" viewportClassName="pr-3" type="auto">
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
                                            style={{ width: `${(header.getSize() / totalColumnSize * 100).toFixed(1)}%` }}
                                            className={cn(
                                                "table-th",
                                                header.column.getCanSort() && "cursor-pointer hover:text-foreground",
                                                align === "right" && "text-right",
                                                align === "center" && "text-center",
                                                isSorted ? "text-foreground" : "text-muted-foreground",
                                            )}
                                            onClick={header.column.getToggleSortingHandler()}
                                        >
                                            <span className="inline-flex items-center gap-1">
                                                {flexRender(header.column.columnDef.header, header.getContext())}
                                                {isSorted === "asc" && <ChevronUp className="size-3" />}
                                                {isSorted === "desc" && <ChevronDown className="size-3" />}
                                            </span>
                                        </TableHead>
                                    );
                                })}
                            </TableRow>
                        ))}
                    </TableHeader>
                    <TableBody>
                        {groups.length > 0 ? (
                            groups.map((group) => (
                                <FormatGroupSection
                                    key={group.label}
                                    group={group}
                                    columnCount={columns.length}
                                    selectedId={selectedId}
                                    mergeId={mergeId}
                                    exprMatches={exprMatches}
                                    onRowClick={onRowClick}
                                />
                            ))
                        ) : (
                            <TableRow className="hover:bg-transparent">
                                <TableCell colSpan={columns.length} className="text-center py-8 text-muted-foreground">
                                    No formats match this filter. Try a different preset or click "Best Quality" to see all.
                                </TableCell>
                            </TableRow>
                        )}
                    </TableBody>
                    <TableFooter className="table-footer-sticky">
                        <TableRow className="hover:bg-transparent">
                            <TableCell colSpan={columns.length} className="table-footer-cell">
                                {filteredCount} format{filteredCount !== 1 ? "s" : ""}
                            </TableCell>
                        </TableRow>
                    </TableFooter>
                </Table>
            </ScrollArea>

            {/* Selection info bar */}
            <div className="px-5 py-2 text-xs text-muted-foreground bg-card border-t border-border shrink-0 flex items-center gap-2 min-h-[36px]">
                {selectedId ? (
                    <>
                        <span>
                            Selected: <strong className="text-foreground/70 font-mono">{selectedId}</strong>
                            {findFormat(selectedId) && (
                                <span className="text-muted-foreground"> ({formatBrief(findFormat(selectedId)!)})</span>
                            )}
                            {mergeId && (
                                <>
                                    {" + "}
                                    <strong className="text-foreground/70 font-mono">{mergeId}</strong>
                                    {findFormat(mergeId) && (
                                        <span className="text-muted-foreground"> ({formatBrief(findFormat(mergeId)!)})</span>
                                    )}
                                </>
                            )}
                        </span>
                        <button
                            className="text-[11px] text-muted-foreground hover:text-foreground underline underline-offset-2 bg-transparent border-none cursor-pointer"
                            onClick={onClearSelection}
                        >
                            Clear
                        </button>
                        {!mergeId && (
                            <span className="text-muted-foreground italic ml-auto text-[11px]">
                                Shift+click to merge
                            </span>
                        )}
                    </>
                ) : (
                    <span className="text-muted-foreground italic">
                        {presetId
                            ? `Using preset: ${PRESETS.find((p) => p.id === presetId)?.label}`
                            : "Click a format to select, or choose a preset above"}
                    </span>
                )}
            </div>

            {/* Expert mode */}
            <div className="px-5 py-2 shrink-0 flex flex-col gap-2 border-t border-border bg-card">
                <div className="flex items-center gap-2">
                    <Checkbox
                        checked={expertMode}
                        onCheckedChange={(checked) => onExpertModeChange(checked === true)}
                        id="expert-mode"
                    />
                    <Label
                        htmlFor="expert-mode"
                        className="text-[11px] font-semibold text-muted-foreground cursor-pointer select-none"
                    >
                        Expert Mode
                    </Label>
                </div>
                {expertMode && (
                    <div className="flex flex-col gap-1 animate-in fade-in-0 slide-in-from-top-1 duration-150">
                        <Input
                            className={cn(
                                "font-mono text-xs h-8",
                                exprError && "border-destructive ring-1 ring-destructive/30",
                            )}
                            type="text"
                            placeholder="e.g. bestvideo[height<=1080]+bestaudio/best"
                            value={expr}
                            onChange={(e) => onExprChange(e.target.value)}
                        />
                        {exprError && (
                            <span className="text-[11px] text-destructive">{exprError}</span>
                        )}
                    </div>
                )}
            </div>
        </>
    );
}
