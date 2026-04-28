// MediaHero: compact card showing thumbnail, title, metadata, and stream badges.
// Adapts display for playlist mode: shows playlist title + episode count.

import { Thumbnail } from "@/components/Thumbnail";
import { StreamBadge } from "@/components/StreamBadge";
import type { FormatListResponse, FormatInfo } from "@/types";

interface MediaHeroProps {
    data: FormatListResponse;
    url: string;
}

function formatDuration(seconds: number): string {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    if (h > 0) {
        return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    }
    return `${m}:${String(s).padStart(2, "0")}`;
}

function getBestFormat(formats: FormatInfo[]): FormatInfo | null {
    const muxed = formats.filter((f) => f.has_video && f.has_audio);
    if (muxed.length > 0) {
        return muxed.sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0] ?? null;
    }
    const videoOnly = formats.filter((f) => f.has_video);
    if (videoOnly.length > 0) {
        return videoOnly.sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0] ?? null;
    }
    return formats[0] ?? null;
}

export function MediaHero({ data, url }: MediaHeroProps) {
    const bestFormat = getBestFormat(data.formats);
    const hostname = (() => {
        try {
            return new URL(url).hostname;
        } catch {
            return url;
        }
    })();

    const isPlaylist = data.is_playlist && data.playlist !== null;

    return (
        <div className="flex items-start gap-3 p-3 border-b border-[#1a1a2e] bg-[var(--surface-raised)] shrink-0">
            {/* Thumbnail */}
            <div className="w-20 h-[52px] shrink-0 rounded-[4px] overflow-hidden bg-[#0e0e1e]">
                <Thumbnail
                    src={data.thumbnail_url}
                    alt={isPlaylist ? (data.playlist?.title ?? data.title) : data.title}
                    className="w-full h-full object-cover"
                />
            </div>

            {/* Metadata */}
            <div className="flex-1 min-w-0 flex flex-col gap-0.5">
                <h1
                    className="text-[14px] font-medium text-[#eeeeee] leading-snug"
                    style={{
                        display: "-webkit-box",
                        WebkitLineClamp: 2,
                        WebkitBoxOrient: "vertical",
                        overflow: "hidden",
                    }}
                >
                    {isPlaylist
                        ? (data.playlist?.title ?? "Untitled Playlist")
                        : (data.title || "Untitled")}
                </h1>

                <div className="flex items-center gap-2 flex-wrap mt-0.5">
                    {isPlaylist ? (
                        <>
                            <span className="text-[11px] text-[#aaaaaa]">
                                {data.playlist!.count} {data.playlist!.count === 1 ? "episode" : "episodes"}
                            </span>
                            <span className="text-[var(--text-muted)]">·</span>
                            <span className="text-[11px] text-[var(--text-muted)]">{hostname}</span>
                        </>
                    ) : (
                        <>
                            <span className="text-[11px] text-[var(--text-muted)]">{hostname}</span>
                            {data.duration !== null && (
                                <>
                                    <span className="text-[var(--text-muted)]">·</span>
                                    <span className="text-[11px] text-[var(--text-muted)]">
                                        {formatDuration(data.duration)}
                                    </span>
                                </>
                            )}
                            {bestFormat?.height && (
                                <>
                                    <span className="text-[var(--text-muted)]">·</span>
                                    <StreamBadge value={`${bestFormat.height}p`} category="resolution" />
                                </>
                            )}
                            {bestFormat?.protocol && (
                                <StreamBadge value={bestFormat.protocol.toUpperCase()} category="protocol" />
                            )}
                            {bestFormat?.vcodec && bestFormat.vcodec !== "none" && (
                                <StreamBadge value={bestFormat.vcodec} category="codec" />
                            )}
                        </>
                    )}
                </div>
            </div>
        </div>
    );
}
