import type { SearchResultPreview } from "../types";

interface ResultCardProps {
    result: SearchResultPreview;
    onDownload: (url: string) => void;
}

/** Format seconds into "M:SS" display string. */
function formatDuration(seconds: number | null): string {
    if (seconds === null) return "";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Format a view count into a human-readable string (e.g. "1.2K views"). */
function formatViews(count: number | null): string {
    if (count === null) return "";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K views`;
    return `${count} views`;
}

/** Displays a single search result as a card with thumbnail, title, and metadata. */
export function ResultCard({ result, onDownload }: ResultCardProps) {
    const duration = formatDuration(result.duration);
    const views = formatViews(result.view_count);

    return (
        <div className="result-card">
            <div className="result-thumbnail">
                {result.thumbnail_url ? (
                    <img
                        src={result.thumbnail_url}
                        alt={result.title}
                        loading="lazy"
                    />
                ) : (
                    <div className="no-thumbnail">
                        No thumbnail
                    </div>
                )}
                {duration && (
                    <span className="duration-badge">{duration}</span>
                )}
                <div className="thumbnail-overlay">
                    <button
                        className="download-btn"
                        onClick={(e) => {
                            e.stopPropagation();
                            onDownload(result.video_url);
                        }}
                    >
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                            <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                            <polyline points="7 10 12 15 17 10" />
                            <line x1="12" y1="15" x2="12" y2="3" />
                        </svg>
                        Download
                    </button>
                </div>
            </div>

            <div className="result-info">
                <h3 className="result-title">{result.title}</h3>
                <div className="result-meta">
                    {views && <span className="result-views">{views}</span>}
                    {views && result.upload_date && (
                        <span className="result-separator">&middot;</span>
                    )}
                    {result.upload_date && (
                        <span className="result-date">
                            {result.upload_date}
                        </span>
                    )}
                </div>
            </div>
        </div>
    );
}
