// Full-screen modal for format selection with hero table, presets, and collapsed options.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
    useReactTable,
    getCoreRowModel,
    getSortedRowModel,
} from "@tanstack/react-table";
import type { SortingState } from "@tanstack/react-table";
import { Loader2, AlertCircle } from "lucide-react";
import { formatsQueryOptions } from "../api/formats";
import { settingsQueryOptions, pickDirectory } from "../api/settings";
import { invokeTyped } from "../api/invokeClient";
import { PRESETS, buildDefaultOptions } from "./FormatOptionsPanel";
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { FormatDialogHeader } from "./FormatDialogHeader";
import { FormatTableSection } from "./FormatTableSection";
import { DownloadOptionsPanel } from "./DownloadOptionsPanel";
import { useFormatColumns } from "./utils/formatColumns";
import { groupRows } from "./utils/tableHelpers";
import type { DownloadOptions } from "../types";

// -- Types ------------------------------------------------------------

interface FormatDialogProps {
    url: string;
    onConfirm: (options: DownloadOptions, title?: string) => void;
    onClose: () => void;
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
    const [sorting, setSorting] = useState<SortingState>([{ id: "height", desc: true }]);
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

    // -- Column definitions -------------------------------------------

    const { columns, totalColumnSize } = useFormatColumns();

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
            setError(queryError.message);
        }
    }, [isQueryError, queryError]);

    // Cleanup debounce timer
    useEffect(() => {
        return () => {
            if (exprTimerRef.current) clearTimeout(exprTimerRef.current);
        };
    }, []);

    // -- Derived data -------------------------------------------------

    const filtered = useMemo(() => {
        const all = formatData?.formats ?? [];
        if (!presetId || presetId === "best") return all;
        if (presetId === "audio-only") return all.filter((f) => !f.has_video);
        if (presetId === "1080p") return all.filter((f) => !f.has_video || (f.height !== null && f.height <= 1080));
        if (presetId === "720p") return all.filter((f) => !f.has_video || (f.height !== null && f.height <= 720));
        return all;
    }, [formatData, presetId]);

    // -- TanStack Table instance ----------------------------------------

    const table = useReactTable({
        data: filtered,
        columns,
        state: { sorting },
        onSortingChange: setSorting,
        getCoreRowModel: getCoreRowModel(),
        getSortedRowModel: getSortedRowModel(),
        enableSortingRemoval: false,
        enableMultiSort: false,
    });

    const groups = groupRows(table.getSortedRowModel().rows);

    const subtitleLangs = useMemo(
        () => formatData?.subtitles.map((s) => s.lang) ?? [],
        [formatData],
    );

    // -- Handlers -----------------------------------------------------

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
        onConfirm({ ...options, format }, formatData?.title);
    }, [buildFormatSelector, onConfirm, options, formatData?.title]);

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
                <FormatDialogHeader
                    title={formatData?.title}
                    thumbnailUrl={formatData?.thumbnail_url}
                />

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
                        <FormatTableSection
                            table={table}
                            groups={groups}
                            columns={columns}
                            totalColumnSize={totalColumnSize}
                            selectedId={selectedId}
                            mergeId={mergeId}
                            exprMatches={exprMatches}
                            presetId={presetId}
                            filteredCount={filtered.length}
                            expertMode={expertMode}
                            expr={expr}
                            exprError={exprError}
                            findFormat={findFormat}
                            onPresetSelect={handlePresetSelect}
                            onRowClick={handleRowClick}
                            onClearSelection={handleClearSelection}
                            onExpertModeChange={setExpertMode}
                            onExprChange={handleExprChange}
                        />
                    )}
                </div>

                {/* -- Zone 3: Collapsed Options + Footer -- */}
                {!isLoading && !error && formatData && (
                    <DownloadOptionsPanel
                        options={options}
                        setOptions={setOptions}
                        settings={settings}
                        subtitleLangs={subtitleLangs}
                        showOptions={showOptions}
                        setShowOptions={setShowOptions}
                        onBrowseDir={() => { handleBrowseDir().catch((e) => console.error("Browse directory failed:", e)); }}
                        onSubLangSelect={handleSubLangSelect}
                    />
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
