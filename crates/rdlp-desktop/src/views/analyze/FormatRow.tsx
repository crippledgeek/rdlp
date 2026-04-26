// FormatRow: single row in the formats table.
// Selected row gets blue left border + tinted background.
// StreamBadge for codec (purple for modern: av1/vp9/h265) and protocol (green).

import { memo } from "react";
import { flexRender } from "@tanstack/react-table";
import type { Row } from "@tanstack/react-table";
import type { FormatInfo } from "@/types";
import { StreamBadge } from "@/components/StreamBadge";
import { cn } from "@/lib/utils";

const MODERN_CODECS = new Set(["av1", "vp9", "h265", "hevc", "vp8"]);
const HLS_PROTOCOLS = new Set(["m3u8", "m3u8_native", "hls", "dash"]);

interface FormatRowProps {
    row: Row<FormatInfo>;
    isSelected: boolean;
    isBest: boolean;
    onClick: (id: string) => void;
}

export const FormatRow = memo(
    function FormatRow({ row, isSelected, isBest, onClick }: FormatRowProps) {
        const format = row.original;

        return (
            <tr
                onClick={() => onClick(format.format_id)}
                data-format-id={format.format_id}
                className={cn(
                    "cursor-pointer transition-colors relative",
                    isSelected
                        ? "bg-[#0f1a2e]"
                        : isBest
                        ? "bg-[#0a110a]"
                        : "hover:bg-[#0e0e1e]",
                )}
                style={{
                    borderBottom: "1px solid #1a1a2e",
                    borderLeft: isSelected
                        ? "3px solid #4a9eff"
                        : isBest
                        ? "3px solid #4a9e4a"
                        : "3px solid transparent",
                }}
            >
                {row.getVisibleCells().map((cell) => {
                    const colId = cell.column.id;

                    // Custom cell rendering for badge columns
                    if (colId === "vcodec") {
                        const codec = format.vcodec;
                        if (!codec || codec === "none") {
                            return (
                                <td key={cell.id} className="px-2 py-1.5 text-[11px] text-[#444444] min-w-[85px]">
                                    —
                                </td>
                            );
                        }
                        const isModern = MODERN_CODECS.has(codec.toLowerCase());
                        return (
                            <td key={cell.id} className="px-2 py-1.5 min-w-[85px]">
                                <StreamBadge
                                    value={codec}
                                    category={isModern ? "codec" : "feature"}
                                />
                            </td>
                        );
                    }

                    if (colId === "acodec") {
                        const audio = format.acodec;
                        const group = format.audio_group_id;
                        // Video-only rows in an HLS master reference an
                        // EXT-X-MEDIA audio group; we render "→ group" so a
                        // user hand-picking can match it to the paired
                        // audio-only row (which renders the same group id).
                        if (!audio || audio === "none") {
                            return (
                                <td key={cell.id} className="px-2 py-1.5 text-[11px] min-w-[90px]">
                                    {group ? (
                                        <span
                                            className="inline-flex items-center gap-0.5 px-1 rounded-[3px] text-[10px] font-mono text-[#7aa57a] bg-[#0f1a0f] border border-[#1f2f1f]"
                                            title={`Pairs with audio rendition group "${group}"`}
                                        >
                                            <span aria-hidden>→</span>
                                            <span>{group}</span>
                                        </span>
                                    ) : (
                                        <span className="text-[#444444]">—</span>
                                    )}
                                </td>
                            );
                        }
                        return (
                            <td key={cell.id} className="px-2 py-1.5 text-[11px] text-[#aaaaaa] min-w-[90px]">
                                <span className="mr-1">{audio}</span>
                                {group && (
                                    <span
                                        className="inline-block px-1 rounded-[3px] text-[10px] font-mono text-[#7aa57a] bg-[#0f1a0f] border border-[#1f2f1f]"
                                        title={`HLS audio rendition group "${group}"`}
                                    >
                                        {group}
                                    </span>
                                )}
                            </td>
                        );
                    }

                    if (colId === "protocol") {
                        const proto = format.protocol;
                        const isHls = HLS_PROTOCOLS.has(proto.toLowerCase());
                        return (
                            <td key={cell.id} className="px-2 py-1.5 min-w-[100px]">
                                <StreamBadge
                                    value={proto.toUpperCase()}
                                    category={isHls ? "protocol" : "feature"}
                                />
                            </td>
                        );
                    }

                    if (colId === "filesize") {
                        return (
                            <td key={cell.id} className="px-2 py-1.5 text-right text-[11px] text-[#aaaaaa] min-w-[75px]">
                                {flexRender(cell.column.columnDef.cell, cell.getContext())}
                            </td>
                        );
                    }

                    return (
                        <td key={cell.id} className="px-2 py-1.5 text-[11px] text-[#aaaaaa]">
                            {flexRender(cell.column.columnDef.cell, cell.getContext())}
                        </td>
                    );
                })}
            </tr>
        );
    },
    (prev, next) =>
        prev.row.id === next.row.id &&
        prev.isSelected === next.isSelected &&
        prev.isBest === next.isBest,
);
