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
            <div className="result-thumbnail-wrapper">
                {result.thumbnail_url ? (
                    <img
                        className="result-thumbnail"
                        src={result.thumbnail_url}
                        alt={result.title}
                        loading="lazy"
                    />
                ) : (
                    <div className="result-thumbnail-placeholder">
                        No thumbnail
                    </div>
                )}
                {duration && (
                    <span className="result-duration-badge">{duration}</span>
                )}
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
                <button
                    className="result-download-button"
                    onClick={() => onDownload(result.video_url)}
                >
                    Download
                </button>
            </div>
        </div>
    );
}
