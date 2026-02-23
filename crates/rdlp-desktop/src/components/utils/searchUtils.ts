// Pure formatting functions for search result display.

/** Format duration in seconds as "m:ss". Returns em-dash for null. */
export function formatDuration(seconds: number | null): string {
    if (seconds === null) return "\u2014";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Format view count as compact string (e.g. "1.2M views", "3.4K views"). */
export function formatViews(count: number | null): string {
    if (count === null) return "";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K views`;
    return `${count} views`;
}
