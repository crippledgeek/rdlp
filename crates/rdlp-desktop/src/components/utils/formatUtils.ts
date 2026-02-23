// Pure formatting functions for FormatInfo display values.
//
// These are stateless helpers with no React dependency, used by
// column definitions and selection info displays.

import type { FormatInfo } from "../../types";

/** Brief description for the selection info bar (e.g. "1080p mp4"). */
export function formatBrief(f: FormatInfo): string {
    const res = f.height ? `${f.height}p` : f.format_note ?? "";
    return `${res} ${f.ext}`.trim();
}

/** Human-readable file size with adaptive units. */
export function formatSize(bytes: number | null): string {
    if (bytes === null) return "\u2014";
    if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
    if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
    if (bytes >= 1_024) return `${(bytes / 1_024).toFixed(0)} KB`;
    return `${bytes} B`;
}

/** Human-readable bitrate with adaptive units. */
export function formatBitrate(kbps: number | null): string {
    if (kbps === null) return "\u2014";
    if (kbps >= 1000) return `${(kbps / 1000).toFixed(1)} Mbps`;
    return `${Math.round(kbps)} kbps`;
}

/** Resolution string: "1920x1080", "1080p", or format_note fallback. */
export function formatResolution(f: FormatInfo): string {
    if (f.height && f.width) return `${f.width}\u00d7${f.height}`;
    if (f.height) return `${f.height}p`;
    return f.format_note ?? "\u2014";
}

/** Type badge label and color class based on protocol and stream presence. */
export function formatType(f: FormatInfo): { label: string; className: string } {
    if (f.protocol.includes("m3u8"))
        return { label: "HLS", className: "bg-yellow-500/10 text-yellow-500" };
    if (f.has_video && f.has_audio)
        return { label: "V+A", className: "bg-primary/10 text-primary" };
    if (f.has_video)
        return { label: "Video", className: "bg-blue-500/10 text-blue-400" };
    return { label: "Audio", className: "bg-purple-500/10 text-purple-400" };
}
