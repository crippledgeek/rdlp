// Pure formatting functions for search result display.

import { formatDistanceToNowStrict } from "date-fns";
import { parseUploadTimestamp } from "@/lib/utils";

/** Format duration in seconds as "m:ss". Returns em-dash for null. */
export function formatDuration(seconds: number | null): string {
    if (seconds === null) return "\u2014";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Format an upload timestamp as compact relative age (e.g. "3d", "2mo", "1y"). */
export function formatCompactDate(timestamp: string | null): string {
    if (timestamp === null) return "";
    const date = parseUploadTimestamp(timestamp);
    if (date === null) return "";
    try {
        return formatDistanceToNowStrict(date, { addSuffix: false })
            .replace(" seconds", "s").replace(" second", "s")
            .replace(" minutes", "m").replace(" minute", "m")
            .replace(" hours", "h").replace(" hour", "h")
            .replace(" days", "d").replace(" day", "d")
            .replace(" weeks", "w").replace(" week", "w")
            .replace(" months", "mo").replace(" month", "mo")
            .replace(" years", "y").replace(" year", "y");
    } catch {
        return "";
    }
}

/** Format view count as compact string (e.g. "1.2M views", "3.4K views"). */
export function formatViews(count: number | null): string {
    if (count === null) return "";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K views`;
    return `${count} views`;
}
