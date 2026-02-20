import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { formatsQueryOptions } from "../api/formats";
import { settingsQueryOptions } from "../api/settings";
import { invokeTyped } from "../api/invokeClient";
import { FormatOptionsPanel, PRESETS } from "./FormatOptionsPanel";
import type {
    DownloadOptions,
    FormatInfo,
} from "../types";

type SortKey = "height" | "ext" | "fps" | "tbr" | "vcodec" | "acodec" | "filesize";
type SortDir = "asc" | "desc";

interface FormatDialogProps {
    url: string;
    onConfirm: (options: DownloadOptions) => void;
    onClose: () => void;
}

/** Format bytes into a human-readable string. */
function formatSize(bytes: number | null): string {
    if (bytes === null) return "-";
    if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
    if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
    if (bytes >= 1_024) return `${(bytes / 1_024).toFixed(0)} KB`;
    return `${bytes} B`;
}

/** Format bitrate (kbps). */
function formatBitrate(kbps: number | null): string {
    if (kbps === null) return "-";
    if (kbps >= 1000) return `${(kbps / 1000).toFixed(1)} Mbps`;
    return `${Math.round(kbps)} kbps`;
}

/** Resolution display string. */
function formatResolution(f: FormatInfo): string {
    if (f.height && f.width) return `${f.width}x${f.height}`;
    if (f.height) return `${f.height}p`;
    return f.format_note ?? "-";
}

/** Type badge label. */
function formatType(f: FormatInfo): { label: string; className: string } {
    if (f.protocol.includes("m3u8")) return { label: "HLS", className: "format-type-hls" };
    if (f.has_video && f.has_audio) return { label: "V+A", className: "format-type-av" };
    if (f.has_video) return { label: "Video", className: "format-type-video" };
    return { label: "Audio", className: "format-type-audio" };
}

/** Compare helper for sorting nullable fields. */
function cmpNullable(a: number | null, b: number | null, dir: SortDir): number {
    if (a === null && b === null) return 0;
    if (a === null) return 1;
    if (b === null) return -1;
    return dir === "asc" ? a - b : b - a;
}

/** Sort formats by a given key and direction. */
function sortFormats(formats: FormatInfo[], key: SortKey, dir: SortDir): FormatInfo[] {
    const sorted = [...formats];
    sorted.sort((a, b) => {
        switch (key) {
            case "height":
                return cmpNullable(a.height, b.height, dir);
            case "ext":
                return dir === "asc"
                    ? a.ext.localeCompare(b.ext)
                    : b.ext.localeCompare(a.ext);
            case "fps":
                return cmpNullable(a.fps, b.fps, dir);
            case "tbr":
                return cmpNullable(a.tbr, b.tbr, dir);
            case "vcodec":
                return dir === "asc"
                    ? (a.vcodec ?? "").localeCompare(b.vcodec ?? "")
                    : (b.vcodec ?? "").localeCompare(a.vcodec ?? "");
            case "acodec":
                return dir === "asc"
                    ? (a.acodec ?? "").localeCompare(b.acodec ?? "")
                    : (b.acodec ?? "").localeCompare(a.acodec ?? "");
            case "filesize":
                return cmpNullable(a.filesize, b.filesize, dir);
            default:
                return 0;
        }
    });
    return sorted;
}

/** Full-screen modal for format selection, presets, options, and expert mode. */
export function FormatDialog({ url, onConfirm, onClose }: FormatDialogProps) {
    // --- TanStack Query: server state ---
    const {
        data: formatData,
        isPending: isLoading,
        isError: isQueryError,
        error: queryError,
        refetch,
    } = useQuery(formatsQueryOptions(url));
    const { data: settings = null } = useQuery(settingsQueryOptions());

    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [mergeId, setMergeId] = useState<string | null>(null);
    const [sortKey, setSortKey] = useState<SortKey>("height");
    const [sortDir, setSortDir] = useState<SortDir>("desc");
    const [showTable, setShowTable] = useState(false);
    const [expertMode, setExpertMode] = useState(false);
    const [expr, setExpr] = useState("");
    const [exprError, setExprError] = useState<string | null>(null);
    const [exprMatches, setExprMatches] = useState<Set<string>>(new Set());
    const [error, setError] = useState<string | null>(null);
    const [options, setOptions] = useState<DownloadOptions>({
        format: null,
        outputDir: null,
        subtitles: (settings?.default_subtitle_langs ?? []).length > 0,
        subtitleLangs: settings?.default_subtitle_langs ?? [],
        remux: settings?.default_remux ?? null,
        extractAudio: settings?.default_extract_audio ?? null,
        embedThumbnail: settings?.embed_thumbnail ?? true,
        audioMultistreams: false,
        recodeVideo: null,
    });

    const dialogRef = useRef<HTMLDivElement>(null);
    const exprTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Surface query-level errors in local state
    useEffect(() => {
        if (isQueryError && queryError) {
            setError(queryError instanceof Error ? queryError.message : String(queryError));
        }
    }, [isQueryError, queryError]);

    // Cleanup timer on unmount
    useEffect(() => {
        return () => {
            if (exprTimerRef.current) clearTimeout(exprTimerRef.current);
        };
    }, []);

    // Close on Escape
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (e.key === "Escape") onClose();
        };
        document.addEventListener("keydown", handler);
        return () => document.removeEventListener("keydown", handler);
    }, [onClose]);

    // Close on backdrop click
    const handleBackdropClick = (e: React.MouseEvent) => {
        if (e.target === e.currentTarget) onClose();
    };

    // Sort header click
    const handleSortClick = (key: SortKey) => {
        if (key === sortKey) {
            setSortDir((d) => (d === "asc" ? "desc" : "asc"));
        } else {
            setSortKey(key);
            setSortDir("desc");
        }
    };

    // Row click (single or shift for merge pair)
    const handleRowClick = (id: string, e: React.MouseEvent) => {
        if (e.shiftKey && selectedId !== null && selectedId !== id) {
            setMergeId(id);
        } else {
            setSelectedId(id);
            setMergeId(null);
        }
    };

    // Preset selection
    const handlePresetSelect = (selector: string) => {
        setSelectedId(null);
        setMergeId(null);
        setOptions((prev) => ({ ...prev, format: selector }));
    };

    // Expert expression change with debounced validation
    const handleExprChange = (value: string) => {
        setExpr(value);
        if (exprTimerRef.current) clearTimeout(exprTimerRef.current);
        if (value.trim() === "") {
            setExprError(null);
            setExprMatches(new Set());
            return;
        }
        exprTimerRef.current = setTimeout(async () => {
            try {
                const fmts = formatData?.formats.map((f) => ({
                    format_id: f.format_id,
                    ext: f.ext,
                    width: f.width,
                    height: f.height,
                    fps: f.fps,
                    tbr: f.tbr,
                    vcodec: f.vcodec,
                    acodec: f.acodec,
                    filesize: f.filesize,
                    vbr: f.vbr,
                    abr: f.abr,
                    asr: f.asr,
                    protocol: f.protocol,
                })) ?? [];
                const matches = await invokeTyped<string[]>(
                    "validate_format_expression",
                    { expression: value, formats: fmts },
                );
                setExprMatches(new Set(matches));
                setExprError(null);
            } catch (err: unknown) {
                const msg = err instanceof Error
                    ? err.message
                    : typeof err === "object" && err !== null && "message" in err
                        ? String((err as { message: unknown }).message)
                        : String(err);
                setExprError(msg);
                setExprMatches(new Set());
            }
        }, 400);
    };

    // Build the final format selector from current state
    const buildFormatSelector = (): string | null => {
        if (expertMode && expr.trim()) return expr.trim();
        if (selectedId && mergeId) return `${selectedId}+${mergeId}`;
        if (selectedId) return selectedId;
        return options.format;
    };

    const handleConfirm = () => {
        const format = buildFormatSelector();
        onConfirm({ ...options, format });
    };

    const sorted = formatData
        ? sortFormats(formatData.formats, sortKey, sortDir)
        : [];

    const subtitleLangs = formatData?.subtitles.map((s) => s.lang) ?? [];

    const renderSortIndicator = (key: SortKey) => {
        if (sortKey !== key) return null;
        return sortDir === "asc" ? " \u25B2" : " \u25BC";
    };

    // Derive active preset from selected state
    const activePresetId = (!selectedId && !mergeId && !expertMode)
        ? PRESETS.find((p) => p.selector === options.format)?.id ?? "best"
        : null;

    return (
        <div className="format-dialog-overlay" onClick={handleBackdropClick}>
            <div className="format-dialog" ref={dialogRef}>
                <div className="format-dialog-header">
                    <span className="format-dialog-title">
                        {formatData?.title ?? "Choose Format"}
                    </span>
                    <button
                        className="format-dialog-close"
                        onClick={onClose}
                        aria-label="Close"
                    >
                        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <line x1="18" y1="6" x2="6" y2="18" />
                            <line x1="6" y1="6" x2="18" y2="18" />
                        </svg>
                    </button>
                </div>

                <div className="format-dialog-body">
                    {isLoading && (
                        <div className="status-message">
                            <div className="loading-dots">
                                <span />
                                <span />
                                <span />
                            </div>
                            <p>Loading formats...</p>
                        </div>
                    )}

                    {error && (
                        <div className="error-banner">
                            <p>{error}</p>
                            <button onClick={() => {
                                setError(null);
                                void refetch();
                            }}>
                                Retry
                            </button>
                        </div>
                    )}

                    {!isLoading && !error && formatData && (
                        <>
                            {/* Preset tier */}
                            <div className="options-popover-section">
                                <div className="options-popover-label">Quality Preset</div>
                                <div className="preset-radio-group">
                                    {PRESETS.map((p) => (
                                        <label key={p.id}>
                                            <input
                                                type="radio"
                                                name="dialog-preset"
                                                checked={activePresetId === p.id}
                                                onChange={() => handlePresetSelect(p.selector)}
                                            />
                                            {p.label}
                                        </label>
                                    ))}
                                </div>
                            </div>

                            {/* Format table toggle */}
                            <button
                                className="filter-toggle"
                                onClick={() => setShowTable((v) => !v)}
                                aria-expanded={showTable}
                            >
                                {showTable ? "Hide all formats" : `Show all formats (${sorted.length})`}
                            </button>

                            {showTable && (
                                <div className="format-table-wrapper">
                                    <table className="format-table">
                                        <thead>
                                            <tr>
                                                <th
                                                    className={`sortable ${sortKey === "height" ? "sorted" : ""}`}
                                                    onClick={() => handleSortClick("height")}
                                                >
                                                    Resolution{renderSortIndicator("height")}
                                                </th>
                                                <th
                                                    className={`sortable ${sortKey === "ext" ? "sorted" : ""}`}
                                                    onClick={() => handleSortClick("ext")}
                                                >
                                                    Ext{renderSortIndicator("ext")}
                                                </th>
                                                <th
                                                    className={`sortable ${sortKey === "fps" ? "sorted" : ""}`}
                                                    onClick={() => handleSortClick("fps")}
                                                >
                                                    FPS{renderSortIndicator("fps")}
                                                </th>
                                                <th
                                                    className={`sortable ${sortKey === "tbr" ? "sorted" : ""}`}
                                                    onClick={() => handleSortClick("tbr")}
                                                >
                                                    Bitrate{renderSortIndicator("tbr")}
                                                </th>
                                                <th
                                                    className={`sortable ${sortKey === "vcodec" ? "sorted" : ""}`}
                                                    onClick={() => handleSortClick("vcodec")}
                                                >
                                                    Video{renderSortIndicator("vcodec")}
                                                </th>
                                                <th
                                                    className={`sortable ${sortKey === "acodec" ? "sorted" : ""}`}
                                                    onClick={() => handleSortClick("acodec")}
                                                >
                                                    Audio{renderSortIndicator("acodec")}
                                                </th>
                                                <th
                                                    className={`sortable ${sortKey === "filesize" ? "sorted" : ""}`}
                                                    onClick={() => handleSortClick("filesize")}
                                                >
                                                    Size{renderSortIndicator("filesize")}
                                                </th>
                                                <th>Type</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {sorted.map((f) => {
                                                const type = formatType(f);
                                                const isSelected =
                                                    f.format_id === selectedId ||
                                                    f.format_id === mergeId;
                                                const isExprMatch = exprMatches.has(f.format_id);
                                                let rowClass = "";
                                                if (isSelected) rowClass += " format-row-selected";
                                                if (isExprMatch) rowClass += " format-row-expr-match";
                                                return (
                                                    <tr
                                                        key={f.format_id}
                                                        className={rowClass}
                                                        onClick={(e) => handleRowClick(f.format_id, e)}
                                                    >
                                                        <td>{formatResolution(f)}</td>
                                                        <td>{f.ext}</td>
                                                        <td>{f.fps !== null ? Math.round(f.fps) : "-"}</td>
                                                        <td>{formatBitrate(f.tbr)}</td>
                                                        <td>{f.vcodec ?? "-"}</td>
                                                        <td>{f.acodec ?? "-"}</td>
                                                        <td>{formatSize(f.filesize)}</td>
                                                        <td>
                                                            <span className={`format-type-badge ${type.className}`}>
                                                                {type.label}
                                                            </span>
                                                        </td>
                                                    </tr>
                                                );
                                            })}
                                        </tbody>
                                    </table>

                                    {selectedId && (
                                        <div className="format-selection-hint">
                                            Selected: <strong>{selectedId}</strong>
                                            {mergeId && (
                                                <> + <strong>{mergeId}</strong> (merge)</>
                                            )}
                                            <span className="hint-text">
                                                {!mergeId && " — Shift+click another format to merge"}
                                            </span>
                                        </div>
                                    )}
                                </div>
                            )}

                            {/* Expert mode */}
                            <label className="expert-toggle">
                                <input
                                    type="checkbox"
                                    checked={expertMode}
                                    onChange={(e) => setExpertMode(e.target.checked)}
                                />
                                Expert Mode
                            </label>

                            {expertMode && (
                                <div className="format-expr-section">
                                    <div className="options-popover-label">Format Expression</div>
                                    <input
                                        className={`format-expr-input ${exprError ? "expr-error" : ""}`}
                                        type="text"
                                        placeholder="e.g. bestvideo[height<=1080]+bestaudio/best"
                                        value={expr}
                                        onChange={(e) => handleExprChange(e.target.value)}
                                    />
                                    {exprError && (
                                        <span className="format-expr-error">{exprError}</span>
                                    )}
                                </div>
                            )}

                            {/* Download options */}
                            <FormatOptionsPanel
                                value={options}
                                onChange={setOptions}
                                availableSubtitleLangs={subtitleLangs}
                                hidePresets
                            />
                        </>
                    )}
                </div>

                <div className="format-dialog-footer">
                    <button
                        className="format-dialog-cancel"
                        onClick={onClose}
                    >
                        Cancel
                    </button>
                    <button
                        className="format-dialog-submit"
                        onClick={handleConfirm}
                        disabled={isLoading || !!error}
                    >
                        Download
                    </button>
                </div>
            </div>
        </div>
    );
}
