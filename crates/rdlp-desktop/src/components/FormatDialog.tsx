import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Loader2, AlertCircle, ChevronUp, ChevronDown } from "lucide-react";
import { formatsQueryOptions } from "../api/formats";
import { settingsQueryOptions } from "../api/settings";
import { invokeTyped } from "../api/invokeClient";
import { FormatOptionsPanel, PRESETS } from "./FormatOptionsPanel";
import { cn } from "@/lib/utils";
import {
    Dialog,
    DialogContent,
    DialogHeader,
    DialogTitle,
    DialogFooter,
    DialogDescription,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
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

/** Type badge label and styling. */
function formatType(f: FormatInfo): { label: string; className: string } {
    if (f.protocol.includes("m3u8"))
        return { label: "HLS", className: "bg-yellow-500/[0.12] text-yellow-500" };
    if (f.has_video && f.has_audio)
        return { label: "V+A", className: "bg-primary/[0.12] text-primary" };
    if (f.has_video)
        return { label: "Video", className: "bg-blue-500/[0.12] text-blue-400" };
    return { label: "Audio", className: "bg-purple-500/[0.12] text-purple-400" };
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
        return sortDir === "asc"
            ? <ChevronUp className="inline size-3" />
            : <ChevronDown className="inline size-3" />;
    };

    // Derive active preset from selected state
    const activePresetId = (!selectedId && !mergeId && !expertMode)
        ? PRESETS.find((p) => p.selector === options.format)?.id ?? "best"
        : null;

    /** Shared cell class for consistent td styling based on row state. */
    const cellClass = (isSelected: boolean, isExprMatch: boolean) => cn(
        "px-2.5 py-1.5 border-b border-white/[0.02] whitespace-nowrap",
        isSelected ? "text-primary" : isExprMatch ? "text-yellow-500" : "text-muted-foreground",
    );

    return (
        <Dialog open={true} onOpenChange={(open) => { if (!open) onClose(); }}>
            <DialogContent
                className="w-[90vw] max-w-[860px] max-h-[85vh] flex flex-col gap-0 p-0"
                showCloseButton={true}
            >
                <DialogHeader className="px-4 pt-4 pb-3 border-b border-border shrink-0">
                    <DialogTitle className="text-[15px] font-bold text-foreground truncate pr-8">
                        {formatData?.title ?? "Choose Format"}
                    </DialogTitle>
                    <DialogDescription className="sr-only">
                        Select a format and download options for this video
                    </DialogDescription>
                </DialogHeader>

                <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-3">
                    {isLoading && (
                        <div className="flex flex-col items-center justify-center gap-2.5 py-8 text-muted-foreground">
                            <Loader2 className="h-6 w-6 animate-spin" />
                            <p className="text-sm">Loading formats...</p>
                        </div>
                    )}

                    {error && (
                        <Alert variant="destructive">
                            <AlertCircle className="h-4 w-4" />
                            <AlertDescription className="flex items-center gap-2.5">
                                <span>{error}</span>
                                <Button
                                    variant="outline"
                                    size="sm"
                                    className="ml-auto h-6 text-[11px]"
                                    onClick={() => {
                                        setError(null);
                                        void refetch();
                                    }}
                                >
                                    Retry
                                </Button>
                            </AlertDescription>
                        </Alert>
                    )}

                    {!isLoading && !error && formatData && (
                        <>
                            {/* Preset tier */}
                            <div className="flex flex-col gap-1">
                                <div className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                                    Quality Preset
                                </div>
                                <div className="flex flex-wrap gap-1">
                                    {PRESETS.map((p) => (
                                        <label
                                            key={p.id}
                                            className={cn(
                                                "inline-flex items-center gap-[5px] px-2.5 py-[5px] border border-white/[0.06] rounded-sm bg-card text-xs text-muted-foreground cursor-pointer transition-colors",
                                                "hover:border-white/[0.12] hover:text-foreground",
                                                activePresetId === p.id && "border-primary text-primary bg-primary/[0.12]",
                                            )}
                                        >
                                            <input
                                                type="radio"
                                                name="dialog-preset"
                                                checked={activePresetId === p.id}
                                                onChange={() => handlePresetSelect(p.selector)}
                                                className={cn(
                                                    "appearance-none w-3 h-3 border-2 border-muted rounded-full bg-muted cursor-pointer shrink-0 relative transition-colors",
                                                    activePresetId === p.id && "border-primary bg-primary after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:w-1 after:h-1 after:rounded-full after:bg-background",
                                                )}
                                            />
                                            {p.label}
                                        </label>
                                    ))}
                                </div>
                            </div>

                            {/* Format table toggle */}
                            <Button
                                variant="ghost"
                                size="sm"
                                className="self-start text-xs text-muted-foreground hover:text-foreground"
                                onClick={() => setShowTable((v) => !v)}
                                aria-expanded={showTable}
                            >
                                {showTable
                                    ? <><ChevronUp className="size-3.5" />Hide all formats</>
                                    : <><ChevronDown className="size-3.5" />Show all formats ({sorted.length})</>}
                            </Button>

                            {showTable && (
                                <div className="flex flex-col border border-border rounded-md animate-in fade-in-0 zoom-in-95">
                                    <ScrollArea className="max-h-[40vh]">
                                        <div className="overflow-x-auto">
                                            <table className="w-full border-collapse font-mono text-xs">
                                                <thead>
                                                    <tr>
                                                        {([
                                                            ["height", "Resolution"],
                                                            ["ext", "Ext"],
                                                            ["fps", "FPS"],
                                                            ["tbr", "Bitrate"],
                                                            ["vcodec", "Video"],
                                                            ["acodec", "Audio"],
                                                            ["filesize", "Size"],
                                                        ] as [SortKey, string][]).map(([key, label]) => (
                                                            <th
                                                                key={key}
                                                                className={cn(
                                                                    "sticky top-0 px-2.5 py-2 text-left text-[10px] font-bold tracking-wider uppercase text-muted-foreground bg-muted border-b border-border whitespace-nowrap select-none cursor-pointer transition-colors hover:text-foreground",
                                                                    sortKey === key && "text-primary",
                                                                )}
                                                                onClick={() => handleSortClick(key)}
                                                            >
                                                                <span className="inline-flex items-center gap-1">
                                                                    {label}
                                                                    {renderSortIndicator(key)}
                                                                </span>
                                                            </th>
                                                        ))}
                                                        <th className="sticky top-0 px-2.5 py-2 text-left text-[10px] font-bold tracking-wider uppercase text-muted-foreground bg-muted border-b border-border whitespace-nowrap select-none">
                                                            Type
                                                        </th>
                                                    </tr>
                                                </thead>
                                                <tbody>
                                                    {sorted.map((f) => {
                                                        const type = formatType(f);
                                                        const isSelected =
                                                            f.format_id === selectedId ||
                                                            f.format_id === mergeId;
                                                        const isExprMatch = exprMatches.has(f.format_id);
                                                        return (
                                                            <tr
                                                                key={f.format_id}
                                                                className={cn(
                                                                    "cursor-pointer transition-colors hover:bg-muted",
                                                                    isSelected && "bg-primary/[0.12]",
                                                                    isExprMatch && !isSelected && "bg-yellow-500/[0.08]",
                                                                )}
                                                                onClick={(e) => handleRowClick(f.format_id, e)}
                                                            >
                                                                <td className={cellClass(isSelected, isExprMatch)}>{formatResolution(f)}</td>
                                                                <td className={cellClass(isSelected, isExprMatch)}>{f.ext}</td>
                                                                <td className={cellClass(isSelected, isExprMatch)}>{f.fps !== null ? Math.round(f.fps) : "-"}</td>
                                                                <td className={cellClass(isSelected, isExprMatch)}>{formatBitrate(f.tbr)}</td>
                                                                <td className={cellClass(isSelected, isExprMatch)}>{f.vcodec ?? "-"}</td>
                                                                <td className={cellClass(isSelected, isExprMatch)}>{f.acodec ?? "-"}</td>
                                                                <td className={cellClass(isSelected, isExprMatch)}>{formatSize(f.filesize)}</td>
                                                                <td className="px-2.5 py-1.5 border-b border-white/[0.02] whitespace-nowrap">
                                                                    <span className={cn(
                                                                        "inline-block px-[7px] py-0.5 rounded-full text-[9px] font-bold tracking-wider uppercase font-mono",
                                                                        type.className,
                                                                    )}>
                                                                        {type.label}
                                                                    </span>
                                                                </td>
                                                            </tr>
                                                        );
                                                    })}
                                                </tbody>
                                            </table>
                                        </div>
                                    </ScrollArea>

                                    {selectedId && (
                                        <div className="px-2.5 py-1.5 text-xs text-muted-foreground bg-muted border-t border-border">
                                            Selected: <strong className="text-primary font-mono">{selectedId}</strong>
                                            {mergeId && (
                                                <> + <strong className="text-primary font-mono">{mergeId}</strong> (merge)</>
                                            )}
                                            <span className="text-muted-foreground italic">
                                                {!mergeId && " \u2014 Shift+click another format to merge"}
                                            </span>
                                        </div>
                                    )}
                                </div>
                            )}

                            {/* Expert mode */}
                            <div className="flex items-center gap-2">
                                <Checkbox
                                    checked={expertMode}
                                    onCheckedChange={(checked) => setExpertMode(checked === true)}
                                    id="expert-mode"
                                />
                                <Label
                                    htmlFor="expert-mode"
                                    className="text-xs font-semibold text-muted-foreground cursor-pointer select-none transition-colors hover:text-foreground"
                                >
                                    Expert Mode
                                </Label>
                            </div>

                            {expertMode && (
                                <div className="flex flex-col gap-1 animate-in fade-in-0 zoom-in-95">
                                    <div className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                                        Format Expression
                                    </div>
                                    <Input
                                        className={cn(
                                            "font-mono text-[13px]",
                                            exprError && "border-destructive ring-1 ring-destructive/30",
                                        )}
                                        type="text"
                                        placeholder="e.g. bestvideo[height<=1080]+bestaudio/best"
                                        value={expr}
                                        onChange={(e) => handleExprChange(e.target.value)}
                                    />
                                    {exprError && (
                                        <span className="text-[11px] text-destructive">{exprError}</span>
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

                <DialogFooter className="px-4 py-2.5 border-t border-border shrink-0">
                    <Button variant="outline" onClick={onClose}>
                        Cancel
                    </Button>
                    <Button
                        onClick={handleConfirm}
                        disabled={isLoading || !!error}
                    >
                        Download
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    );
}
