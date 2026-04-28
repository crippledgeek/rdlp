// ResultCard: single search result card.
// Shows thumbnail, title, duration, view count, upload date.
// Click triggers analyzeUrl to extract formats.

import { Thumbnail } from "@/components/Thumbnail";
import { setAnalyzeUrl } from "@/stores/uiStore";
import { cn } from "@/lib/utils";
import type { SearchResultPreview } from "@/types";

interface ResultCardProps {
    result: SearchResultPreview;
    className?: string;
}

function formatDuration(seconds: number | null): string | null {
    if (seconds === null) return null;
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    return `${m}:${String(s).padStart(2, "0")}`;
}

function formatViewCount(count: number | null): string | null {
    if (count === null) return null;
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(0)}K views`;
    return `${count} views`;
}

export function ResultCard({ result, className }: ResultCardProps) {
    function handleClick() {
        setAnalyzeUrl(result.video_url);
    }

    const duration = formatDuration(result.duration);
    const viewCount = formatViewCount(result.view_count);

    return (
        <button
            onClick={handleClick}
            className={cn(
                "flex gap-2.5 p-2 rounded-[6px] text-left w-full hover:bg-[#0e0e1e] transition-colors cursor-pointer",
                className,
            )}
        >
            {/* Thumbnail */}
            <div className="w-28 h-16 shrink-0 rounded-[4px] overflow-hidden bg-[#0a0a1a] relative">
                <Thumbnail
                    src={result.thumbnail_url}
                    alt={result.title}
                    className="w-full h-full object-cover"
                />
                {duration && (
                    <span className="absolute bottom-0.5 right-0.5 px-1 py-0 rounded-[2px] bg-black/70 text-white text-[10px] font-mono">
                        {duration}
                    </span>
                )}
            </div>

            {/* Metadata */}
            <div className="flex-1 min-w-0 flex flex-col gap-0.5">
                <p
                    className="text-[12px] font-medium text-[#eeeeee] leading-snug"
                    style={{
                        display: "-webkit-box",
                        WebkitLineClamp: 2,
                        WebkitBoxOrient: "vertical",
                        overflow: "hidden",
                    }}
                >
                    {result.title}
                </p>
                <div className="flex items-center gap-1.5 flex-wrap mt-0.5">
                    {viewCount && (
                        <span className="text-[10px] text-[var(--text-muted)]">{viewCount}</span>
                    )}
                    {result.upload_date && (
                        <span className="text-[10px] text-[var(--text-muted)]">{result.upload_date}</span>
                    )}
                </div>
            </div>
        </button>
    );
}
