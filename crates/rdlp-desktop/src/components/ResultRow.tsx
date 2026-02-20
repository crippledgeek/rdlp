// Single row in the dense list view. Shows thumbnail, title, metadata columns, and actions.

import { useState, useRef, useEffect } from "react";
import { formatDistanceToNowStrict, fromUnixTime } from "date-fns";
import { useSettingsStore } from "../lib/store";
import { FormatOptionsPanel } from "./FormatOptionsPanel";
import type {
    AudioFormat,
    ContainerFormat,
    DownloadOptions,
    SearchResultPreview,
} from "../types";

interface ResultRowProps {
    result: SearchResultPreview;
    focused: boolean;
    selected: boolean;
    onDownload: (url: string) => void;
    onDownloadWithOptions: (url: string, options: Partial<DownloadOptions>) => void;
    onOpenFormatDialog: (url: string) => void;
    onToggleSelect: () => void;
}

function formatDuration(seconds: number | null): string {
    if (seconds === null) return "\u2014";
    const m = Math.floor(seconds / 60);
    const s = Math.floor(seconds % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
}

function formatCompactDate(timestamp: string | null): string {
    if (timestamp === null) return "";
    const ts = Number(timestamp);
    if (Number.isNaN(ts)) return "";
    try {
        return formatDistanceToNowStrict(fromUnixTime(ts), { addSuffix: false })
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

function formatViews(count: number | null): string {
    if (count === null) return "";
    if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M views`;
    if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K views`;
    return `${count} views`;
}

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

/** Dense list row for a single search result. */
export function ResultRow({
    result,
    focused,
    selected,
    onDownload,
    onDownloadWithOptions,
    onOpenFormatDialog: _onOpenFormatDialog,
    onToggleSelect,
}: ResultRowProps) {
    const settings = useSettingsStore((s) => s.settings);
    const [showOptions, setShowOptions] = useState(false);
    const [panelOptions, setPanelOptions] = useState<DownloadOptions>(() =>
        buildOptionsFromSettings(settings),
    );
    const popoverRef = useRef<HTMLDivElement>(null);
    const rowRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        if (!showOptions) {
            setPanelOptions(buildOptionsFromSettings(settings));
        }
    }, [settings, showOptions]);

    useEffect(() => {
        if (!showOptions) return;
        const handler = (e: MouseEvent) => {
            if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
                setShowOptions(false);
            }
        };
        document.addEventListener("mousedown", handler);
        return () => document.removeEventListener("mousedown", handler);
    }, [showOptions]);

    // Scroll into view when focused via keyboard
    useEffect(() => {
        if (focused && rowRef.current) {
            rowRef.current.scrollIntoView({ block: "nearest" });
        }
    }, [focused]);

    const duration = formatDuration(result.duration);
    const views = formatViews(result.view_count);
    const date = formatCompactDate(result.upload_date);

    return (
        <div
            ref={rowRef}
            className={[
                "result-row",
                focused ? "result-row-focused" : "",
                selected ? "result-row-selected" : "",
            ].join(" ")}
            data-focused={focused || undefined}
        >
            {selected && (
                <div className="result-row-checkbox" onClick={onToggleSelect}>
                    <svg viewBox="0 0 16 16" width="12" height="12" fill="var(--accent)">
                        <rect x="1" y="1" width="14" height="14" rx="2" fill="none" stroke="var(--accent)" strokeWidth="1.5" />
                        <path d="M4 8l3 3 5-6" fill="none" stroke="var(--accent)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                </div>
            )}

            <div className="result-row-thumb">
                {result.thumbnail_url ? (
                    <img src={result.thumbnail_url} alt="" loading="lazy" />
                ) : (
                    <div className="result-row-thumb-placeholder" />
                )}
            </div>

            <div className="result-row-info">
                <div className="result-row-title" title={result.title}>
                    {result.title}
                </div>
                {views && (
                    <div className="result-row-meta">{views}</div>
                )}
            </div>

            <div className="result-row-col result-row-duration">{duration}</div>
            <div className="result-row-col result-row-date">{date}</div>

            <div className="result-row-actions">
                <button
                    className="result-row-action"
                    onClick={(e) => { e.stopPropagation(); onDownload(result.video_url); }}
                    title="Download"
                    aria-label="Download"
                >
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                        <polyline points="7 10 12 15 17 10" />
                        <line x1="12" y1="15" x2="12" y2="3" />
                    </svg>
                </button>
                <button
                    className="result-row-action"
                    onClick={(e) => { e.stopPropagation(); setShowOptions((v) => !v); }}
                    title="Download options"
                    aria-label="Download options"
                >
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M6 9l6 6 6-6" />
                    </svg>
                </button>
            </div>

            {showOptions && (
                <div className="result-row-popover" ref={popoverRef} onClick={(e) => e.stopPropagation()}>
                    <FormatOptionsPanel value={panelOptions} onChange={setPanelOptions} />
                    <button
                        className="options-popover-confirm"
                        onClick={() => {
                            onDownloadWithOptions(result.video_url, panelOptions);
                            setShowOptions(false);
                        }}
                    >
                        Download with Options
                    </button>
                </div>
            )}
        </div>
    );
}
