// TanStack Table column definitions for the formats table.
// Columns: Quality, Resolution, FPS, VCodec, Audio, Protocol, Size.

import { createColumnHelper } from "@tanstack/react-table";
import type { FormatInfo } from "@/types";

const helper = createColumnHelper<FormatInfo>();

function nullsLastSort(
    a: number | null,
    b: number | null,
    asc: boolean,
): number {
    if (a === null && b === null) return 0;
    if (a === null) return 1; // nulls always last
    if (b === null) return -1;
    return asc ? a - b : b - a;
}

export function getQualityLabel(format: FormatInfo): string {
    if (!format.has_video) return "Audio";
    const h = format.height;
    if (!h) return format.format_note ?? "Unknown";
    if (h >= 2160) return "4K";
    if (h >= 1440) return "2K";
    if (h >= 1080) return "1080p";
    if (h >= 720) return "720p";
    if (h >= 480) return "480p";
    if (h >= 360) return "360p";
    return `${h}p`;
}

export const formatColumns = [
    helper.accessor((row) => getQualityLabel(row), {
        id: "quality",
        header: "Quality",
        size: 80,
        enableSorting: true,
        sortingFn: (rowA, rowB) => {
            // Sort by height descending for quality
            const ha = rowA.original.height ?? 0;
            const hb = rowB.original.height ?? 0;
            return hb - ha;
        },
    }),
    helper.accessor("height", {
        header: "Resolution",
        size: 80,
        enableSorting: true,
        sortingFn: (rowA, rowB) => {
            return nullsLastSort(rowA.original.height, rowB.original.height, false);
        },
        cell: (info) => {
            const h = info.getValue();
            const w = info.row.original.width;
            if (!h) return "—";
            return w ? `${w}×${h}` : `${h}p`;
        },
    }),
    helper.accessor("fps", {
        header: "FPS",
        size: 45,
        enableSorting: true,
        sortingFn: (rowA, rowB) =>
            nullsLastSort(rowA.original.fps, rowB.original.fps, false),
        cell: (info) => info.getValue() ?? "—",
    }),
    helper.accessor("vcodec", {
        header: "VCodec",
        size: 60,
        enableSorting: true,
        cell: (info) => {
            const v = info.getValue();
            if (!v || v === "none") return "—";
            return v;
        },
    }),
    helper.accessor("acodec", {
        header: "Audio",
        // Slightly wider than before to fit the optional HLS audio-group tag
        // that FormatRow appends when a variant references an EXT-X-MEDIA
        // rendition. Falls back to "—" / "<codec>" on rows without a group.
        size: 70,
        enableSorting: true,
        cell: (info) => {
            const a = info.getValue();
            if (!a || a === "none") return "—";
            return a;
        },
    }),
    helper.accessor("protocol", {
        header: "Protocol",
        size: 55,
        enableSorting: true,
    }),
    helper.accessor("filesize", {
        header: "Size",
        size: 65,
        enableSorting: true,
        sortingFn: (rowA, rowB) =>
            nullsLastSort(rowA.original.filesize, rowB.original.filesize, false),
        cell: (info) => {
            const bytes = info.getValue();
            if (bytes === null) return "—";
            if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
            if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(0)} MB`;
            if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
            return `${bytes} B`;
        },
    }),
];
