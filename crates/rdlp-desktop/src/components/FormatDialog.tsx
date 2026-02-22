import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
    Loader2,
    AlertCircle,
    ChevronUp,
    ChevronDown,
    FolderOpen,
    Settings2,
} from "lucide-react";
import { formatsQueryOptions } from "../api/formats";
import { settingsQueryOptions, pickDirectory } from "../api/settings";
import { invokeTyped } from "../api/invokeClient";
import { PRESETS, buildDefaultOptions } from "./FormatOptionsPanel";
import { FormatGroupSection, type FormatGroup } from "./FormatGroupSection";
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
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import {
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "@/components/ui/popover";
import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
} from "@/components/ui/collapsible";
import {
    Table,
    TableBody,
    TableCell,
    TableHead,
    TableHeader,
    TableRow,
} from "@/components/ui/table";
import {
    ToggleGroup,
    ToggleGroupItem,
} from "@/components/ui/toggle-group";
import type {
    AudioFormat,
    ContainerFormat,
    DownloadOptions,
    FormatInfo,
} from "../types";

// -- Types ------------------------------------------------------------

type SortKey = "height" | "ext" | "fps" | "tbr" | "vcodec" | "acodec" | "filesize";
type SortDir = "asc" | "desc";

interface FormatDialogProps {
    url: string;
    onConfirm: (options: DownloadOptions) => void;
    onClose: () => void;
}

// -- Constants --------------------------------------------------------

const COLUMNS: Array<{ key: SortKey; label: string; align: "left" | "right"; pct: string }> = [
    { key: "height", label: "Resolution", align: "left", pct: "22%" },
    { key: "ext", label: "Ext", align: "left", pct: "7%" },
    { key: "fps", label: "FPS", align: "right", pct: "7%" },
    { key: "tbr", label: "Bitrate", align: "right", pct: "13%" },
    { key: "vcodec", label: "Video", align: "left", pct: "12%" },
    { key: "acodec", label: "Audio", align: "left", pct: "11%" },
    { key: "filesize", label: "Size", align: "right", pct: "13%" },
];

const REMUX_OPTIONS: Array<{ value: ContainerFormat; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
];

const AUDIO_OPTIONS: Array<{ value: AudioFormat; label: string }> = [
    { value: "mp3", label: "MP3" },
    { value: "aac", label: "AAC" },
    { value: "opus", label: "Opus" },
    { value: "flac", label: "FLAC" },
];

const NONE_SENTINEL = "none";

// -- Utility functions ------------------------------------------------

/** Brief description for the selection info bar. */
function formatBrief(f: FormatInfo): string {
    const res = f.height ? `${f.height}p` : f.format_note ?? "";
    return `${res} ${f.ext}`.trim();
}

function cmpNullable(a: number | null, b: number | null, dir: SortDir): number {
    if (a === null && b === null) return 0;
    if (a === null) return 1;
    if (b === null) return -1;
    return dir === "asc" ? a - b : b - a;
}

function sortFormats(formats: FormatInfo[], key: SortKey, dir: SortDir): FormatInfo[] {
    const sorted = [...formats];
    sorted.sort((a, b) => {
        switch (key) {
            case "height": return cmpNullable(a.height, b.height, dir);
            case "ext": return dir === "asc" ? a.ext.localeCompare(b.ext) : b.ext.localeCompare(a.ext);
            case "fps": return cmpNullable(a.fps, b.fps, dir);
            case "tbr": return cmpNullable(a.tbr, b.tbr, dir);
            case "vcodec": return dir === "asc" ? (a.vcodec ?? "").localeCompare(b.vcodec ?? "") : (b.vcodec ?? "").localeCompare(a.vcodec ?? "");
            case "acodec": return dir === "asc" ? (a.acodec ?? "").localeCompare(b.acodec ?? "") : (b.acodec ?? "").localeCompare(a.acodec ?? "");
            case "filesize": return cmpNullable(a.filesize, b.filesize, dir);
            default: return 0;
        }
    });
    return sorted;
}

/** Group sorted formats into Video+Audio, Video Only, Audio Only sections. */
function groupFormats(formats: FormatInfo[]): FormatGroup[] {
    const videoAudio: FormatInfo[] = [];
    const videoOnly: FormatInfo[] = [];
    const audioOnly: FormatInfo[] = [];

    for (const f of formats) {
        if (f.has_video && f.has_audio) videoAudio.push(f);
        else if (f.has_video) videoOnly.push(f);
        else audioOnly.push(f);
    }

    const groups: FormatGroup[] = [];
    if (videoAudio.length > 0) groups.push({ label: "Video + Audio", formats: videoAudio });
    if (videoOnly.length > 0) groups.push({ label: "Video Only", formats: videoOnly });
    if (audioOnly.length > 0) groups.push({ label: "Audio Only", formats: audioOnly });
    return groups;
}

/** Build a summary string for the collapsed options trigger. */
function optionsSummary(opts: DownloadOptions): string {
    const parts: string[] = [];
    if (opts.remux) parts.push(`${opts.remux.toUpperCase()} remux`);
    if (opts.extractAudio) parts.push(`${opts.extractAudio.toUpperCase()} audio`);
    if (opts.subtitles && opts.subtitleLangs.length > 0)
        parts.push(`Subs: ${opts.subtitleLangs.join(", ")}`);
    if (opts.embedThumbnail) parts.push("Thumbnail");
    return parts.length > 0 ? parts.join(" \u00b7 ") : "Default settings";
}

/** Full-screen modal for format selection with hero table, presets, and collapsed options. */
export function FormatDialog({ url, onConfirm, onClose }: FormatDialogProps) {
    // -- Server state -------------------------------------------------
    const {
        data: formatData,
        isPending: isLoading,
        isError: isQueryError,
        error: queryError,
        refetch,
    } = useQuery(formatsQueryOptions(url));
    const { data: settings = null } = useQuery(settingsQueryOptions());

    // -- Local state --------------------------------------------------
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [mergeId, setMergeId] = useState<string | null>(null);
    const [sortKey, setSortKey] = useState<SortKey>("height");
    const [sortDir, setSortDir] = useState<SortDir>("desc");
    const [expertMode, setExpertMode] = useState(false);
    const [expr, setExpr] = useState("");
    const [exprError, setExprError] = useState<string | null>(null);
    const [exprMatches, setExprMatches] = useState<Set<string>>(new Set());
    const [error, setError] = useState<string | null>(null);
    const [settingsApplied, setSettingsApplied] = useState(false);
    const [showOptions, setShowOptions] = useState(false);
    const [presetId, setPresetId] = useState<string | null>(null);
    const [options, setOptions] = useState<DownloadOptions>(() =>
        buildDefaultOptions(settings),
    );

    const exprTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const tableRef = useRef<HTMLDivElement>(null);

    // -- Effects ------------------------------------------------------

    // Sync options when settings first load
    useEffect(() => {
        if (settings && !settingsApplied) {
            setOptions((prev) => ({
                ...prev,
                subtitles: (settings.default_subtitle_langs ?? []).length > 0,
                subtitleLangs: settings.default_subtitle_langs ?? [],
                remux: settings.default_remux ?? prev.remux,
                extractAudio: settings.default_extract_audio ?? prev.extractAudio,
                embedThumbnail: settings.embed_thumbnail ?? prev.embedThumbnail,
            }));
            setSettingsApplied(true);
        }
    }, [settings, settingsApplied]);

    // Surface query errors in local state
    useEffect(() => {
        if (isQueryError && queryError) {
            setError(queryError instanceof Error ? queryError.message : String(queryError));
        }
    }, [isQueryError, queryError]);

    // Cleanup debounce timer
    useEffect(() => {
        return () => {
            if (exprTimerRef.current) clearTimeout(exprTimerRef.current);
        };
    }, []);

    // -- Derived data -------------------------------------------------

    const sorted = useMemo(
        () => (formatData ? sortFormats(formatData.formats, sortKey, sortDir) : []),
        [formatData, sortKey, sortDir],
    );

    const filtered = useMemo(() => {
        if (!presetId || presetId === "best") return sorted;
        if (presetId === "audio-only") return sorted.filter((f) => !f.has_video);
        if (presetId === "1080p") return sorted.filter((f) => !f.has_video || (f.height !== null && f.height <= 1080));
        if (presetId === "720p") return sorted.filter((f) => !f.has_video || (f.height !== null && f.height <= 720));
        return sorted;
    }, [sorted, presetId]);

    const groups = useMemo(() => groupFormats(filtered), [filtered]);

    const subtitleLangs = useMemo(
        () => formatData?.subtitles.map((s) => s.lang) ?? [],
        [formatData],
    );

    // -- Handlers -----------------------------------------------------

    const handleSortClick = useCallback((key: SortKey) => {
        setSortKey((prev) => {
            if (prev === key) {
                setSortDir((d) => (d === "asc" ? "desc" : "asc"));
                return prev;
            }
            setSortDir("desc");
            return key;
        });
    }, []);

    const handleRowClick = useCallback((id: string, e: React.MouseEvent) => {
        if (e.shiftKey && selectedId !== null && selectedId !== id) {
            setMergeId(id);
        } else {
            setSelectedId(id);
            setMergeId(null);
        }
        // Clear preset when manually selecting
        setPresetId(null);
        setOptions((prev) => ({ ...prev, format: null }));
    }, [selectedId]);

    const handlePresetSelect = useCallback((newPresetId: string) => {
        if (!newPresetId) {
            setPresetId(null);
            setOptions((prev) => ({ ...prev, format: null }));
            return;
        }
        const preset = PRESETS.find((p) => p.id === newPresetId);
        if (!preset) return;
        setPresetId(newPresetId);
        setSelectedId(null);
        setMergeId(null);
        setOptions((prev) => ({ ...prev, format: preset.selector }));
    }, []);

    const handleClearSelection = useCallback(() => {
        setSelectedId(null);
        setMergeId(null);
    }, []);

    const handleExprChange = useCallback((value: string) => {
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
                    format_id: f.format_id, ext: f.ext, width: f.width,
                    height: f.height, fps: f.fps, tbr: f.tbr,
                    vcodec: f.vcodec, acodec: f.acodec, filesize: f.filesize,
                    vbr: f.vbr, abr: f.abr, asr: f.asr, protocol: f.protocol,
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
    }, [formatData]);

    const buildFormatSelector = useCallback((): string | null => {
        if (expertMode && expr.trim()) return expr.trim();
        if (selectedId && mergeId) return `${selectedId}+${mergeId}`;
        if (selectedId) return selectedId;
        return options.format;
    }, [expertMode, expr, selectedId, mergeId, options.format]);

    const handleConfirm = useCallback(() => {
        const format = buildFormatSelector();
        onConfirm({ ...options, format });
    }, [buildFormatSelector, onConfirm, options]);

    const handleBrowseDir = useCallback(async () => {
        const dir = await pickDirectory();
        if (dir) setOptions((prev) => ({ ...prev, outputDir: dir }));
    }, []);

    const handleSubLangSelect = useCallback((lang: string) => {
        setOptions((prev) => {
            const current = prev.subtitleLangs;
            const next = current.includes(lang)
                ? current.filter((l) => l !== lang)
                : [...current, lang];
            return { ...prev, subtitleLangs: next };
        });
    }, []);

    // -- Lookup helpers -----------------------------------------------

    const findFormat = useCallback(
        (id: string) => formatData?.formats.find((f) => f.format_id === id),
        [formatData],
    );

    // -- Render -------------------------------------------------------

    return (
        <Dialog open={true} onOpenChange={(open) => { if (!open) onClose(); }}>
            <DialogContent
                className="bg-card w-[90vw] max-w-[900px] max-h-[85vh] flex flex-col gap-0 p-0 overflow-hidden"
                showCloseButton={true}
            >
                {/* -- Zone 1: Context Header -- */}
                <DialogHeader className="px-5 pt-4 pb-3 border-b border-border shrink-0">
                    <div className="flex items-center gap-3">
                        {formatData?.thumbnail_url && (
                            <img
                                src={formatData.thumbnail_url}
                                alt=""
                                className="w-12 h-[27px] object-cover rounded-sm shrink-0 bg-muted"
                            />
                        )}
                        <div className="min-w-0 flex-1">
                            <DialogTitle className="text-sm font-semibold text-foreground truncate pr-8">
                                {formatData?.title ?? "Choose Format"}
                            </DialogTitle>
                            <DialogDescription className="sr-only">
                                Select a format and download options for this video
                            </DialogDescription>
                        </div>
                    </div>
                </DialogHeader>

                {/* -- Zone 2: Hero Format Table -- */}
                <div className="flex-1 min-h-0 flex flex-col">
                    {isLoading && (
                        <div className="flex flex-col items-center justify-center gap-2.5 py-12 text-muted-foreground">
                            <Loader2 className="size-5 animate-spin" />
                            <p className="text-xs">Loading formats...</p>
                        </div>
                    )}

                    {error && (
                        <div className="p-4">
                            <Alert variant="destructive">
                                <AlertCircle className="size-4" />
                                <AlertDescription className="flex items-center gap-2.5">
                                    <span className="text-xs">{error}</span>
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        className="ml-auto h-6 text-[11px]"
                                        onClick={() => {
                                            setError(null);
                                            refetch().catch((e) => console.error("Refetch failed:", e));
                                        }}
                                    >
                                        Retry
                                    </Button>
                                </AlertDescription>
                            </Alert>
                        </div>
                    )}

                    {!isLoading && !error && formatData && (
                        <>
                            {/* Preset segmented control */}
                            <div className="px-5 pt-3 pb-2 shrink-0">
                                <ToggleGroup
                                    type="single"
                                    value={presetId ?? ""}
                                    onValueChange={handlePresetSelect}
                                    className="justify-start gap-0 bg-muted rounded-md p-0.5"
                                >
                                    {PRESETS.map((p) => (
                                        <ToggleGroupItem
                                            key={p.id}
                                            value={p.id}
                                            className="px-3 h-7 text-xs rounded-[5px] text-muted-foreground hover:text-foreground hover:bg-foreground/[0.04] data-[state=on]:bg-foreground/10 data-[state=on]:text-foreground data-[state=on]:shadow-sm"
                                        >
                                            {p.label}
                                        </ToggleGroupItem>
                                    ))}
                                </ToggleGroup>
                            </div>

                            {/* Scrollable table */}
                            <ScrollArea className="flex-1 min-h-0" viewportClassName="pr-3" type="auto" ref={tableRef}>
                                <Table className="text-xs" style={{ tableLayout: "fixed" }}>
                                    <TableHeader className="sticky top-0 z-10">
                                        <TableRow className="bg-foreground/[0.06] backdrop-blur-sm hover:bg-foreground/[0.06]">
                                            {COLUMNS.map(({ key, label, align, pct }) => (
                                                <TableHead
                                                    key={key}
                                                    style={{ width: pct }}
                                                    className={cn(
                                                        "h-8 px-3 text-[10px] font-bold tracking-wider uppercase select-none cursor-pointer transition-colors hover:text-foreground",
                                                        align === "right" && "text-right",
                                                        sortKey === key ? "text-foreground" : "text-muted-foreground",
                                                    )}
                                                    onClick={() => handleSortClick(key)}
                                                >
                                                    <span className="inline-flex items-center gap-1">
                                                        {label}
                                                        {sortKey === key && (
                                                            sortDir === "asc"
                                                                ? <ChevronUp className="size-3" />
                                                                : <ChevronDown className="size-3" />
                                                        )}
                                                    </span>
                                                </TableHead>
                                            ))}
                                            <TableHead style={{ width: "15%" }} className="h-8 px-3 text-[10px] font-bold tracking-wider uppercase text-muted-foreground select-none text-center">
                                                Type
                                            </TableHead>
                                        </TableRow>
                                    </TableHeader>
                                    <TableBody>
                                        {groups.length > 0 ? (
                                            groups.map((group) => (
                                                <FormatGroupSection
                                                    key={group.label}
                                                    group={group}
                                                    selectedId={selectedId}
                                                    mergeId={mergeId}
                                                    exprMatches={exprMatches}
                                                    onRowClick={handleRowClick}
                                                />
                                            ))
                                        ) : (
                                            <TableRow className="hover:bg-transparent">
                                                <TableCell colSpan={8} className="text-center py-8 text-muted-foreground">
                                                    No formats match this filter. Try a different preset or click "Best Quality" to see all.
                                                </TableCell>
                                            </TableRow>
                                        )}
                                    </TableBody>
                                </Table>
                            </ScrollArea>

                            {/* Selection info bar */}
                            <div className="px-5 py-2 text-xs text-muted-foreground bg-card border-t border-border shrink-0 flex items-center gap-2 min-h-[36px]">
                                {selectedId ? (
                                    <>
                                        <span>
                                            Selected: <strong className="text-foreground/70 font-mono">{selectedId}</strong>
                                            {findFormat(selectedId) && (
                                                <span className="text-muted-foreground"> ({formatBrief(findFormat(selectedId)!)})</span>
                                            )}
                                            {mergeId && (
                                                <>
                                                    {" + "}
                                                    <strong className="text-foreground/70 font-mono">{mergeId}</strong>
                                                    {findFormat(mergeId) && (
                                                        <span className="text-muted-foreground"> ({formatBrief(findFormat(mergeId)!)})</span>
                                                    )}
                                                </>
                                            )}
                                        </span>
                                        <button
                                            className="text-[11px] text-muted-foreground hover:text-foreground underline underline-offset-2 bg-transparent border-none cursor-pointer"
                                            onClick={handleClearSelection}
                                        >
                                            Clear
                                        </button>
                                        {!mergeId && (
                                            <span className="text-muted-foreground italic ml-auto text-[11px]">
                                                Shift+click to merge
                                            </span>
                                        )}
                                    </>
                                ) : (
                                    <span className="text-muted-foreground italic">
                                        {presetId
                                            ? `Using preset: ${PRESETS.find((p) => p.id === presetId)?.label}`
                                            : "Click a format to select, or choose a preset above"}
                                    </span>
                                )}
                            </div>

                            {/* Expert mode */}
                            <div className="px-5 py-2 shrink-0 flex flex-col gap-2 border-t border-border bg-card">
                                <div className="flex items-center gap-2">
                                    <Checkbox
                                        checked={expertMode}
                                        onCheckedChange={(checked) => setExpertMode(checked === true)}
                                        id="expert-mode"
                                    />
                                    <Label
                                        htmlFor="expert-mode"
                                        className="text-[11px] font-semibold text-muted-foreground cursor-pointer select-none"
                                    >
                                        Expert Mode
                                    </Label>
                                </div>
                                {expertMode && (
                                    <div className="flex flex-col gap-1 animate-in fade-in-0 slide-in-from-top-1 duration-150">
                                        <Input
                                            className={cn(
                                                "font-mono text-xs h-8",
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
                            </div>
                        </>
                    )}
                </div>

                {/* -- Zone 3: Collapsed Options + Footer -- */}
                {!isLoading && !error && formatData && (
                    <div className="border-t border-border shrink-0">
                        <Collapsible open={showOptions} onOpenChange={setShowOptions}>
                            <CollapsibleTrigger asChild>
                                <button className="w-full px-5 py-2 flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground transition-colors cursor-pointer bg-transparent border-none">
                                    <Settings2 className="size-3.5" />
                                    <span className="font-semibold">Download Options</span>
                                    <span className="text-muted-foreground/60 ml-1 truncate">
                                        {!showOptions && optionsSummary(options)}
                                    </span>
                                    {showOptions
                                        ? <ChevronUp className="size-3.5 ml-auto shrink-0" />
                                        : <ChevronDown className="size-3.5 ml-auto shrink-0" />}
                                </button>
                            </CollapsibleTrigger>
                            <CollapsibleContent>
                                <div className="px-5 pb-3 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2.5 items-center animate-in fade-in-0 slide-in-from-top-1 duration-150">
                                    {/* Save to */}
                                    <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                                        Save to
                                    </Label>
                                    <div className="flex gap-1.5">
                                        <Input
                                            className="flex-1 text-xs h-7 font-mono"
                                            type="text"
                                            readOnly
                                            value={options.outputDir ?? settings?.output_dir ?? ""}
                                            placeholder="Default directory"
                                        />
                                        <Button
                                            variant="outline"
                                            size="sm"
                                            className="h-7 px-2 shrink-0 text-xs"
                                            onClick={() => { handleBrowseDir().catch((e) => console.error("Browse directory failed:", e)); }}
                                        >
                                            <FolderOpen className="size-3" />
                                        </Button>
                                    </div>

                                    {/* Remux */}
                                    <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                                        Remux
                                    </Label>
                                    <Select
                                        value={options.remux ?? NONE_SENTINEL}
                                        onValueChange={(val) => setOptions((prev) => ({
                                            ...prev,
                                            remux: val === NONE_SENTINEL ? null : (val as ContainerFormat),
                                        }))}
                                    >
                                        <SelectTrigger className="h-7 text-xs">
                                            <SelectValue />
                                        </SelectTrigger>
                                        <SelectContent>
                                            <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                                            {REMUX_OPTIONS.map((o) => (
                                                <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                            ))}
                                        </SelectContent>
                                    </Select>

                                    {/* Extract Audio */}
                                    <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                                        Audio
                                    </Label>
                                    <Select
                                        value={options.extractAudio ?? NONE_SENTINEL}
                                        onValueChange={(val) => setOptions((prev) => ({
                                            ...prev,
                                            extractAudio: val === NONE_SENTINEL ? null : (val as AudioFormat),
                                        }))}
                                    >
                                        <SelectTrigger className="h-7 text-xs">
                                            <SelectValue />
                                        </SelectTrigger>
                                        <SelectContent>
                                            <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                                            {AUDIO_OPTIONS.map((o) => (
                                                <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                            ))}
                                        </SelectContent>
                                    </Select>

                                    {/* Subtitles */}
                                    <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                                        Subtitles
                                    </Label>
                                    <div className="flex flex-col gap-1.5">
                                        <div className="flex items-center gap-2">
                                            <Checkbox
                                                checked={options.subtitles}
                                                onCheckedChange={(checked) => setOptions((prev) => ({
                                                    ...prev,
                                                    subtitles: checked === true,
                                                }))}
                                                id="dialog-subtitles"
                                            />
                                            <Label htmlFor="dialog-subtitles" className="text-xs text-muted-foreground cursor-pointer">
                                                Download subtitles
                                            </Label>
                                        </div>
                                        {options.subtitles && (
                                            <div className="animate-in fade-in-0 duration-150">
                                                {subtitleLangs.length > 0 ? (
                                                    <Popover>
                                                        <PopoverTrigger asChild>
                                                            <Button variant="outline" size="sm" className="w-full justify-between text-xs font-normal h-7">
                                                                <span className="truncate">
                                                                    {options.subtitleLangs.length > 0
                                                                        ? options.subtitleLangs.join(", ")
                                                                        : "Select languages"}
                                                                </span>
                                                            </Button>
                                                        </PopoverTrigger>
                                                        <PopoverContent align="start" className="w-(--radix-popover-trigger-width) p-2">
                                                            <div className="flex flex-col gap-0.5 max-h-48 overflow-y-auto">
                                                                {subtitleLangs.map((lang) => (
                                                                    <Label
                                                                        key={lang}
                                                                        htmlFor={`dialog-sub-${lang}`}
                                                                        className="flex items-center gap-2 rounded-sm px-2 py-1 text-xs cursor-pointer hover:bg-accent"
                                                                    >
                                                                        <Checkbox
                                                                            id={`dialog-sub-${lang}`}
                                                                            checked={options.subtitleLangs.includes(lang)}
                                                                            onCheckedChange={() => handleSubLangSelect(lang)}
                                                                        />
                                                                        {lang}
                                                                    </Label>
                                                                ))}
                                                            </div>
                                                        </PopoverContent>
                                                    </Popover>
                                                ) : (
                                                    <Input
                                                        className="font-mono text-xs h-7"
                                                        type="text"
                                                        placeholder="en,sv,ja"
                                                        value={options.subtitleLangs.join(",")}
                                                        onChange={(e) => {
                                                            const langs = e.target.value.split(",").map((s) => s.trim()).filter(Boolean);
                                                            setOptions((prev) => ({ ...prev, subtitleLangs: langs }));
                                                        }}
                                                    />
                                                )}
                                            </div>
                                        )}
                                    </div>

                                    {/* Thumbnail */}
                                    <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                                        Thumbnail
                                    </Label>
                                    <div className="flex items-center gap-2">
                                        <Checkbox
                                            checked={options.embedThumbnail}
                                            onCheckedChange={(checked) => setOptions((prev) => ({
                                                ...prev,
                                                embedThumbnail: checked === true,
                                            }))}
                                            id="dialog-thumbnail"
                                        />
                                        <Label htmlFor="dialog-thumbnail" className="text-xs text-muted-foreground cursor-pointer">
                                            Embed thumbnail
                                        </Label>
                                    </div>
                                </div>
                            </CollapsibleContent>
                        </Collapsible>
                    </div>
                )}

                <DialogFooter className="px-5 py-2.5 border-t border-border shrink-0">
                    <Button variant="outline" size="sm" onClick={onClose}>
                        Cancel
                    </Button>
                    <Button size="sm" onClick={handleConfirm} disabled={isLoading || !!error}>
                        Download
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    );
}

