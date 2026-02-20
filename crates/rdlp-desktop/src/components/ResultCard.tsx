import { useEffect, useRef, useState } from "react";
import { formatDistanceToNow, fromUnixTime } from "date-fns";
import { useQuery } from "@tanstack/react-query";
import { settingsQueryOptions } from "../api/settings";
import { FormatOptionsPanel } from "./FormatOptionsPanel";
import type {
    AudioFormat,
    ContainerFormat,
    DownloadOptions,
    SearchResultPreview,
} from "../types";

interface ResultCardProps {
    result: SearchResultPreview;
    onDownload: (url: string) => void;
    onDownloadWithOptions: (url: string, options: Partial<DownloadOptions>) => void;
    onOpenFormatDialog: (url: string) => void;
}

/** Format seconds into "M:SS" display string. */
function formatDuration(seconds: number | null): string {
    if (seconds === null) return "";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Format a Unix timestamp string into a relative date (e.g. "3 days ago"). */
function formatUploadDate(timestamp: string | null): string {
    if (timestamp === null) return "";
    const ts = Number(timestamp);
    if (Number.isNaN(ts)) return "";
    return formatDistanceToNow(fromUnixTime(ts), { addSuffix: true });
}

/** Format a view count into a human-readable string (e.g. "1.2K views"). */
function formatViews(count: number | null): string {
    if (count === null) return "";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K views`;
    return `${count} views`;
}

/** Build default DownloadOptions from settings for the popover initial state. */
function buildOptionsFromSettings(settings: {
    default_remux: ContainerFormat | null;
    default_extract_audio: AudioFormat | null;
    default_subtitle_langs: string[];
    embed_thumbnail: boolean;
} | null): DownloadOptions {
    return {
        format: null,
        outputDir: null,
        subtitles: (settings?.default_subtitle_langs ?? []).length > 0,
        subtitleLangs: settings?.default_subtitle_langs ?? [],
        remux: settings?.default_remux ?? null,
        extractAudio: settings?.default_extract_audio ?? null,
        embedThumbnail: settings?.embed_thumbnail ?? true,
        audioMultistreams: false,
        recodeVideo: null,
    };
}

/** Displays a single search result as a card with thumbnail, title, and metadata. */
export function ResultCard({
    result,
    onDownload,
    onDownloadWithOptions,
    onOpenFormatDialog,
}: ResultCardProps) {
    const duration = formatDuration(result.duration);
    const views = formatViews(result.view_count);
    const uploadDate = formatUploadDate(result.upload_date);
    const { data: settings = null } = useQuery(settingsQueryOptions());

    const [showOptions, setShowOptions] = useState(false);
    const [panelOptions, setPanelOptions] = useState<DownloadOptions>(() =>
        buildOptionsFromSettings(settings),
    );
    const popoverRef = useRef<HTMLDivElement>(null);

    // Update panel defaults when settings change
    useEffect(() => {
        if (!showOptions) {
            setPanelOptions(buildOptionsFromSettings(settings));
        }
    }, [settings, showOptions]);

    // Dismiss popover on outside click
    useEffect(() => {
        if (!showOptions) return;
        const handler = (e: MouseEvent) => {
            if (
                popoverRef.current &&
                !popoverRef.current.contains(e.target as Node)
            ) {
                setShowOptions(false);
            }
        };
        document.addEventListener("mousedown", handler);
        return () => document.removeEventListener("mousedown", handler);
    }, [showOptions]);

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
                    <div className="download-actions">
                        <div className="download-btn-group">
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
                            <button
                                className="download-btn download-btn-options"
                                onClick={(e) => {
                                    e.stopPropagation();
                                    setShowOptions((v) => !v);
                                }}
                                aria-label="Download options"
                            >
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                                    <path d="M6 9l6 6 6-6" />
                                </svg>
                            </button>
                        </div>
                        <button
                            className="choose-format-btn"
                            onClick={(e) => {
                                e.stopPropagation();
                                onOpenFormatDialog(result.video_url);
                            }}
                        >
                            Choose Format
                        </button>
                    </div>

                    {showOptions && (
                        <div
                            className="options-popover"
                            ref={popoverRef}
                            onClick={(e) => e.stopPropagation()}
                        >
                            <FormatOptionsPanel
                                value={panelOptions}
                                onChange={setPanelOptions}
                            />
                            <button
                                className="options-popover-confirm"
                                onClick={() => {
                                    onDownloadWithOptions(
                                        result.video_url,
                                        panelOptions,
                                    );
                                    setShowOptions(false);
                                }}
                            >
                                Download with Options
                            </button>
                        </div>
                    )}
                </div>
            </div>

            <div className="result-info">
                <h3 className="result-title">{result.title}</h3>
                <div className="result-meta">
                    {views && <span className="result-views">{views}</span>}
                    {views && uploadDate && (
                        <span className="result-separator">&middot;</span>
                    )}
                    {uploadDate && (
                        <span className="result-date">
                            {uploadDate}
                        </span>
                    )}
                </div>
            </div>
        </div>
    );
}
