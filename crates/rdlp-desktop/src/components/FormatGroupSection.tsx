import { cn } from "@/lib/utils";
import {
    TableCell,
    TableRow,
} from "@/components/ui/table";
import type { FormatInfo } from "../types";

// -- Types ----------------------------------------------------------------

export interface FormatGroup {
    label: string;
    formats: FormatInfo[];
}

interface FormatGroupSectionProps {
    group: FormatGroup;
    selectedId: string | null;
    mergeId: string | null;
    exprMatches: Set<string>;
    onRowClick: (id: string, e: React.MouseEvent) => void;
}

// -- Utility functions ----------------------------------------------------

function formatSize(bytes: number | null): string {
    if (bytes === null) return "\u2014";
    if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
    if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
    if (bytes >= 1_024) return `${(bytes / 1_024).toFixed(0)} KB`;
    return `${bytes} B`;
}

function formatBitrate(kbps: number | null): string {
    if (kbps === null) return "\u2014";
    if (kbps >= 1000) return `${(kbps / 1000).toFixed(1)} Mbps`;
    return `${Math.round(kbps)} kbps`;
}

function formatResolution(f: FormatInfo): string {
    if (f.height && f.width) return `${f.width}\u00d7${f.height}`;
    if (f.height) return `${f.height}p`;
    return f.format_note ?? "\u2014";
}

function formatType(f: FormatInfo): { label: string; className: string } {
    if (f.protocol.includes("m3u8"))
        return { label: "HLS", className: "bg-yellow-500/10 text-yellow-500" };
    if (f.has_video && f.has_audio)
        return { label: "V+A", className: "bg-primary/10 text-primary" };
    if (f.has_video)
        return { label: "Video", className: "bg-blue-500/10 text-blue-400" };
    return { label: "Audio", className: "bg-purple-500/10 text-purple-400" };
}

/** Renders a section header + rows for a format group (Video+Audio, Video Only, Audio Only). */
export function FormatGroupSection({ group, selectedId, mergeId, exprMatches, onRowClick }: FormatGroupSectionProps) {
    return (
        <>
            {/* Section header */}
            <TableRow className="hover:bg-transparent bg-transparent border-none">
                <TableCell
                    colSpan={8}
                    className="px-3 py-1.5 text-[10px] font-bold uppercase tracking-widest text-foreground/40"
                >
                    {group.label}
                </TableCell>
            </TableRow>

            {/* Format rows */}
            {group.formats.map((f, i) => {
                const type = formatType(f);
                const isSelected = f.format_id === selectedId;
                const isMerge = f.format_id === mergeId;
                const isExprMatch = exprMatches.has(f.format_id);
                const isEven = i % 2 === 0;

                return (
                    <TableRow
                        key={f.format_id}
                        className={cn(
                            "cursor-pointer transition-colors border-b-foreground/[0.06] hover:bg-foreground/[0.06]",
                            isEven ? "bg-transparent" : "bg-foreground/[0.02]",
                            isSelected && "bg-primary/20 border-l-[3px] border-l-primary",
                            isMerge && "bg-foreground/10 border-l-[3px] border-l-foreground/40",
                            isExprMatch && !isSelected && !isMerge && "bg-yellow-500/5",
                        )}
                        onClick={(e) => onRowClick(f.format_id, e)}
                    >
                        <TableCell className={cn(
                            "px-3 font-mono",
                            isSelected || isMerge ? "text-foreground" : isExprMatch ? "text-yellow-500" : "text-foreground/60",
                        )}>
                            {formatResolution(f)}
                        </TableCell>
                        <TableCell className={cn(
                            "px-3",
                            isSelected || isMerge ? "text-foreground" : isExprMatch ? "text-yellow-500" : "text-foreground/60",
                        )}>
                            {f.ext}
                        </TableCell>
                        <TableCell className={cn(
                            "px-3 text-right font-mono",
                            isSelected || isMerge ? "text-foreground" : isExprMatch ? "text-yellow-500" : "text-foreground/60",
                        )}>
                            {f.fps !== null ? Math.round(f.fps) : "\u2014"}
                        </TableCell>
                        <TableCell className={cn(
                            "px-3 text-right font-mono",
                            isSelected || isMerge ? "text-foreground" : isExprMatch ? "text-yellow-500" : "text-foreground/60",
                        )}>
                            {formatBitrate(f.tbr)}
                        </TableCell>
                        <TableCell className={cn(
                            "px-3",
                            isSelected || isMerge ? "text-foreground" : isExprMatch ? "text-yellow-500" : "text-foreground/60",
                        )}>
                            {f.vcodec ?? "\u2014"}
                        </TableCell>
                        <TableCell className={cn(
                            "px-3",
                            isSelected || isMerge ? "text-foreground" : isExprMatch ? "text-yellow-500" : "text-foreground/60",
                        )}>
                            {f.acodec ?? "\u2014"}
                        </TableCell>
                        <TableCell className={cn(
                            "px-3 text-right font-mono",
                            isSelected || isMerge ? "text-foreground" : isExprMatch ? "text-yellow-500" : "text-foreground/60",
                        )}>
                            {formatSize(f.filesize)}
                        </TableCell>
                        <TableCell className="px-3 text-center">
                            <span className={cn(
                                "inline-block px-1.5 py-0.5 rounded text-[9px] font-bold tracking-wider uppercase",
                                type.className,
                            )}>
                                {type.label}
                            </span>
                        </TableCell>
                    </TableRow>
                );
            })}
        </>
    );
}
