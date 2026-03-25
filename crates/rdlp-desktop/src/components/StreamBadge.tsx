// Badge component for streaming metadata: protocol, resolution, codec, feature.
// Auto-detects category from value, or accepts an explicit `category` prop.

import { cn } from "@/lib/utils";

export type BadgeCategory = "protocol" | "resolution" | "codec" | "feature" | "status";

interface StreamBadgeProps {
    value: string;
    category?: BadgeCategory;
    className?: string;
}

function detectCategory(value: string): BadgeCategory {
    const lower = value.toLowerCase();
    // Protocol
    if (["hls", "https", "http", "dash", "m3u8", "frag"].some((p) => lower.includes(p))) {
        return "protocol";
    }
    // Resolution
    if (/^\d{3,4}p$/.test(lower) || ["2160", "1440", "1080", "720", "480", "360"].some((r) => lower.includes(r))) {
        return "resolution";
    }
    // Codec
    if (["av1", "vp9", "h265", "hevc", "h264", "avc", "aac", "opus", "mp3", "flac", "vorbis"].some((c) => lower.includes(c))) {
        return "codec";
    }
    // Feature
    if (["subtitles", "chapters", "hdr", "sdr", "dolby", "dts", "atmos"].some((f) => lower.includes(f))) {
        return "feature";
    }
    return "protocol";
}

const categoryStyles: Record<BadgeCategory, string> = {
    protocol: "text-[#4a9e4a] bg-[#1a3020]",
    resolution: "text-[#6a8aff] bg-[#1a2040]",
    codec: "text-[#c084fc] bg-[#2a1a30]",
    feature: "text-[#5abfbf] bg-[#1a2020]",
    status: "text-[#aaaaaa] bg-[#1a1a2e]",
};

export function StreamBadge({ value, category, className }: StreamBadgeProps) {
    const cat = category ?? detectCategory(value);
    return (
        <span
            className={cn(
                "inline-flex items-center px-[5px] py-[1px] rounded-[3px] text-[10px] font-medium uppercase tracking-wide whitespace-nowrap",
                categoryStyles[cat],
                className,
            )}
        >
            {value}
        </span>
    );
}
