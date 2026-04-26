// DownloadConfig: config panel content for the analyze view when a format is selected.
// Uses TanStack Form + Zod for form state and validation.

import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { useForm } from "@tanstack/react-form";
import { toast } from "@/components/ui/sonner";
import { FolderOpen, Download } from "lucide-react";
import { z } from "zod";
import { codecsQueryOptions } from "@/api/codecs";
import { audioCodecsQueryOptions } from "@/api/audioCodecs";
import { uiStore, setView } from "@/stores/uiStore";
import { formatsQueryOptions } from "@/api/formats";
import { settingsQueryOptions, pickDirectory } from "@/api/settings";
import { startDownload } from "@/api/downloads";
import { StreamBadge } from "@/components/StreamBadge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
    Select,
    SelectItem,
    SelectListBox,
    SelectPopover,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { DownloadOptions } from "@/types";

const NONE_SENTINEL = "none";

// Codec → default container + encoder mapping
const CODEC_MAP: Record<string, { container: string; encoder: string }> = {
    h264: { container: "mp4", encoder: "libx264" },
    h265: { container: "mp4", encoder: "libx265" },
    vp9: { container: "webm", encoder: "libvpx-vp9" },
    av1: { container: "mkv", encoder: "libsvtav1" },
};

function formatFileSize(bytes: number | null): string {
    if (bytes === null) return "Unknown";
    if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
    if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(0)} MB`;
    return `${(bytes / 1024).toFixed(0)} KB`;
}

// Zod schema for the download config form
const downloadConfigSchema = z.object({
    outputPath: z.string(),
    remux: z.string(),
    recodeCodec: z.string(),
    recodeContainerOverride: z.string(),
    videoEncoder: z.string(),
    recodeAudioMode: z.string(),
    extractAudio: z.string(),
    embedThumbnail: z.boolean(),
    embedSubtitles: z.boolean(),
    normalizeAudio: z.boolean(),
    expertMode: z.boolean(),
    verboseMode: z.boolean(),
});

type DownloadConfigValues = z.infer<typeof downloadConfigSchema>;

export function DownloadConfig() {
    const analyzeUrl = useStore(uiStore, (s) => s.analyzeUrl);
    const selectedFormatId = useStore(uiStore, (s) => s.selectedFormatId);
    const selectedSelector = useStore(uiStore, (s) => s.selectedSelector);
    // A selection exists when either a single format row is picked OR an
    // auto-pair preset (which emits a DSL string) is active. The two fields
    // are mutually exclusive per `uiStore` setters.
    const hasSelection = selectedFormatId !== null || selectedSelector !== null;

    const { data: formatData } = useQuery(formatsQueryOptions(analyzeUrl));
    const { data: settings } = useQuery(settingsQueryOptions());

    const form = useForm({
        defaultValues: {
            outputPath: "",
            remux: "",
            recodeCodec: "",
            recodeContainerOverride: "",
            videoEncoder: "",
            recodeAudioMode: "copy",
            extractAudio: "",
            embedThumbnail: settings?.embed_thumbnail ?? true,
            embedSubtitles: settings?.embed_subtitles ?? false,
            normalizeAudio: settings?.normalize_audio ?? false,
            expertMode: false as boolean,
            verboseMode: false as boolean,
        } satisfies DownloadConfigValues,
        onSubmit: async ({ value }) => {
            if (!analyzeUrl || !hasSelection) return;
            // Selector override wins if present (e.g. the "Best" preset
            // resolved to `bv*+ba/best`); otherwise send the single
            // `format_id` the user picked from the formats table.
            const formatArg = selectedSelector ?? selectedFormatId;
            if (!formatArg) return;

            const codecInfo = CODEC_MAP[value.recodeCodec];
            const isExpertEncoder = value.recodeCodec !== "" && !codecInfo;
            const resolvedContainer = value.recodeContainerOverride || codecInfo?.container || "";
            const resolvedEncoder = value.videoEncoder || codecInfo?.encoder || (isExpertEncoder ? value.recodeCodec : "");

            try {
                await startDownload(analyzeUrl, {
                    format: formatArg,
                    outputDir: value.outputPath || settings?.output_dir || null,
                    subtitles: false,
                    subtitleLangs: [],
                    remux: (value.remux || null) as DownloadOptions["remux"],
                    extractAudio: (value.extractAudio || null) as DownloadOptions["extractAudio"],
                    embedThumbnail: value.embedThumbnail,
                    audioMultistreams: false,
                    recodeVideo: (resolvedContainer || null) as DownloadOptions["recodeVideo"],
                    videoEncoder: resolvedEncoder || null,
                    recodeContainer: null,
                    recodeAudio: value.recodeAudioMode === "copy"
                        ? { mode: "copy" as const }
                        : value.recodeAudioMode === "auto"
                        ? { mode: "auto" as const }
                        : value.recodeAudioMode
                        ? { mode: "encoder" as const, name: value.recodeAudioMode }
                        : null,
                    normalizeAudio: value.normalizeAudio || null,
                    loudnorm: null,
                    loudnormPreset: null,
                    loudnormTargetI: null,
                    loudnormTargetTp: null,
                    loudnormTargetLra: null,
                    loudnormDynamic: null,
                    loudnormPrecompress: null,
                    normalizeBoost: null,
                    normalizeBoostDb: null,
                    embedSubtitles: value.embedSubtitles || null,
                    verbose: value.verboseMode || null,
                }, formatData?.title ?? undefined);
                toast.success("Download queued");
                setView("queue");
            } catch (err: unknown) {
                const message = err && typeof err === "object" && "message" in err
                    ? String((err as { message: unknown }).message)
                    : "Failed to start download";
                toast.error(message);
            }
        },
    });

    // Derive computed values from form state for conditional rendering
    const recodeCodec = useStore(form.store, (s) => s.values.recodeCodec);
    const recodeContainerOverride = useStore(form.store, (s) => s.values.recodeContainerOverride);
    const expertMode = useStore(form.store, (s) => s.values.expertMode);
    const isSubmitting = useStore(form.store, (s) => s.isSubmitting);

    const recodeActive = recodeCodec !== "";
    const codecInfo = CODEC_MAP[recodeCodec];
    const resolvedContainer = recodeContainerOverride || codecInfo?.container || "";

    // Fetch available codecs for expert mode
    const { data: videoCodecs = [] } = useQuery({
        ...codecsQueryOptions(),
        enabled: expertMode,
    });
    const { data: audioCodecs = [] } = useQuery({
        ...audioCodecsQueryOptions(resolvedContainer || null),
        enabled: recodeActive && !!resolvedContainer,
    });

    if (!formatData || !analyzeUrl) return null;

    const selectedFormat = formatData.formats.find(
        (f) => f.format_id === selectedFormatId,
    );

    // Preview rows that the auto-pair selector will pick. Used to render a
    // helpful summary when `selectedSelector` is active (no single format
    // row is highlighted in that mode).
    const autoPairPreview = selectedSelector
        ? {
              video: [...formatData.formats]
                  .filter((f) => f.has_video && !f.has_audio)
                  .sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0] ?? null,
              audio: formatData.formats.find((f) => !f.has_video && f.has_audio) ?? null,
          }
        : null;

    return (
        <form
            className="flex flex-col h-full overflow-y-auto gap-0"
            onSubmit={(e) => {
                e.preventDefault();
                void form.handleSubmit();
            }}
        >
            {/* Selected format summary */}
            <section className="p-3 border-b border-[#1a1a2e]">
                <h3 className="text-[10px] font-bold uppercase tracking-widest text-[#666666] mb-2">
                    Selected Format
                </h3>
                {selectedFormat ? (
                    <div className="flex flex-col gap-1.5">
                        <div className="flex items-center gap-1.5 flex-wrap">
                            {selectedFormat.height && (
                                <StreamBadge value={`${selectedFormat.height}p`} category="resolution" />
                            )}
                            {selectedFormat.vcodec && selectedFormat.vcodec !== "none" && (
                                <StreamBadge value={selectedFormat.vcodec} category="codec" />
                            )}
                            {selectedFormat.protocol && (
                                <StreamBadge value={selectedFormat.protocol.toUpperCase()} category="protocol" />
                            )}
                        </div>
                        <div className="flex items-center gap-3 text-[11px] text-[#666666]">
                            {selectedFormat.fps && <span>{selectedFormat.fps} fps</span>}
                            {selectedFormat.filesize !== null && (
                                <span>{formatFileSize(selectedFormat.filesize)}</span>
                            )}
                        </div>
                    </div>
                ) : selectedSelector ? (
                    <div className="flex flex-col gap-1.5">
                        <div className="flex items-center gap-1.5 flex-wrap">
                            <StreamBadge value="AUTO PAIR" category="protocol" />
                            {autoPairPreview?.video?.height && (
                                <StreamBadge
                                    value={`${autoPairPreview.video.height}p`}
                                    category="resolution"
                                />
                            )}
                            {autoPairPreview?.video?.vcodec &&
                                autoPairPreview.video.vcodec !== "none" && (
                                    <StreamBadge
                                        value={autoPairPreview.video.vcodec}
                                        category="codec"
                                    />
                                )}
                            {autoPairPreview?.audio?.acodec &&
                                autoPairPreview.audio.acodec !== "none" && (
                                    <StreamBadge
                                        value={autoPairPreview.audio.acodec}
                                        category="codec"
                                    />
                                )}
                        </div>
                        <p className="text-[11px] text-[#666666]">
                            Best video + best audio, merged via FFmpeg.
                        </p>
                    </div>
                ) : (
                    <p className="text-[11px] text-[#444444]">No format selected</p>
                )}
            </section>

            {/* Output path */}
            <section className="p-3 border-b border-[#1a1a2e]">
                <h3 className="text-[10px] font-bold uppercase tracking-widest text-[#666666] mb-2">
                    Output
                </h3>
                <form.Field name="outputPath">
                    {(field) => (
                        <div className="flex items-center gap-1">
                            <Input
                                type="text"
                                value={field.state.value || settings?.output_dir || ""}
                                onBlur={field.handleBlur}
                                onChange={(e) => field.handleChange(e.target.value)}
                                placeholder="Default output directory"
                                className="flex-1 min-w-0 px-2 py-1 rounded-[4px] bg-[var(--surface-elevated)] border border-[#2a2a3e] text-[11px] text-[#aaaaaa] placeholder:text-[#444444] outline-none focus:border-[#4a9eff]"
                            />
                            <Button
                                type="button"
                                variant="ghost"
                                onPress={async () => {
                                    const dir = await pickDirectory();
                                    if (dir) field.handleChange(dir);
                                }}
                                className="p-1.5 rounded-[4px] bg-[var(--surface-elevated)] border border-[#2a2a3e] text-[#666666] hover:text-[#aaaaaa] hover:border-[#3a3a4e] transition-colors"
                                aria-label="Browse for output directory"
                            >
                                <FolderOpen className="w-3.5 h-3.5" />
                            </Button>
                        </div>
                    )}
                </form.Field>
            </section>

            {/* Post-Processing */}
            <section className="p-3 border-b border-[#1a1a2e]">
                <div className="flex items-center justify-between mb-2">
                    <h3 className="text-[10px] font-bold uppercase tracking-widest text-[#666666]">
                        Post-Processing
                    </h3>
                    <div className="flex items-center gap-3">
                        <form.Field name="expertMode">
                            {(field) => (
                                <Checkbox
                                    isSelected={field.state.value}
                                    onChange={field.handleChange}
                                    className="gap-1 text-[10px] text-[#555555]"
                                >
                                    Expert
                                </Checkbox>
                            )}
                        </form.Field>
                        <form.Field name="verboseMode">
                            {(field) => (
                                <Checkbox
                                    isSelected={field.state.value}
                                    onChange={field.handleChange}
                                    className="gap-1 text-[10px] text-[#555555]"
                                >
                                    Verbose
                                </Checkbox>
                            )}
                        </form.Field>
                    </div>
                </div>
                <div className="flex flex-col gap-2">
                    {/* Remux */}
                    <form.Field name="remux">
                        {(field) => (
                            <div className="flex items-center justify-between">
                                <span className="text-[11px] text-[#aaaaaa]">Remux</span>
                                <Select
                                    selectedKey={field.state.value || NONE_SENTINEL}
                                    onSelectionChange={(key) => field.handleChange(key === NONE_SENTINEL ? "" : String(key))}
                                    aria-label="Remux container"
                                >
                                    <SelectTrigger className="h-6 min-h-0 px-2 py-0 text-[11px] bg-[var(--surface-elevated)] border-[#2a2a3e] rounded-[4px] text-[#aaaaaa] w-[90px]">
                                        <SelectValue />
                                    </SelectTrigger>
                                    <SelectPopover>
                                        <SelectListBox>
                                            <SelectItem id={NONE_SENTINEL} textValue="Auto">Auto</SelectItem>
                                            <SelectItem id="mp4" textValue="MP4">MP4</SelectItem>
                                            <SelectItem id="mkv" textValue="MKV">MKV</SelectItem>
                                            <SelectItem id="webm" textValue="WebM">WebM</SelectItem>
                                            <SelectItem id="mov" textValue="MOV">MOV</SelectItem>
                                        </SelectListBox>
                                    </SelectPopover>
                                </Select>
                            </div>
                        )}
                    </form.Field>

                    {/* Recode Video — codec selection */}
                    <form.Field name="recodeCodec">
                        {(field) => (
                            <div className="flex items-center justify-between">
                                <span className="text-[11px] text-[#aaaaaa]">Recode</span>
                                <Select
                                    selectedKey={field.state.value || NONE_SENTINEL}
                                    onSelectionChange={(key) => {
                                        const val = key === NONE_SENTINEL ? "" : String(key);
                                        field.handleChange(val);
                                        if (!val) {
                                            form.setFieldValue("videoEncoder", "");
                                            form.setFieldValue("recodeContainerOverride", "");
                                            form.setFieldValue("recodeAudioMode", "copy");
                                        }
                                    }}
                                    aria-label="Recode video codec"
                                >
                                    <SelectTrigger className="h-6 min-h-0 px-2 py-0 text-[11px] bg-[var(--surface-elevated)] border-[#2a2a3e] rounded-[4px] text-[#aaaaaa] w-[90px]">
                                        <SelectValue />
                                    </SelectTrigger>
                                    <SelectPopover>
                                        <SelectListBox>
                                            <SelectItem id={NONE_SENTINEL} textValue="None">None</SelectItem>
                                            {expertMode && videoCodecs.length > 0
                                                ? videoCodecs.flatMap((c) =>
                                                    c.encoders.map((enc) => (
                                                        <SelectItem key={enc.encoderName} id={enc.encoderName} textValue={enc.displayName}>
                                                            {enc.displayName}
                                                        </SelectItem>
                                                    ))
                                                )
                                                : <>
                                                    <SelectItem id="h264" textValue="H.264 / AVC">H.264 / AVC</SelectItem>
                                                    <SelectItem id="h265" textValue="H.265 / HEVC">H.265 / HEVC</SelectItem>
                                                    <SelectItem id="vp9" textValue="VP9">VP9</SelectItem>
                                                    <SelectItem id="av1" textValue="AV1">AV1</SelectItem>
                                                </>
                                            }
                                        </SelectListBox>
                                    </SelectPopover>
                                </Select>
                            </div>
                        )}
                    </form.Field>

                    {/* Container override (visible when recode active) */}
                    {recodeActive && (
                        <form.Field name="recodeContainerOverride">
                            {(field) => (
                                <div className="flex items-center justify-between pl-3">
                                    <span className="text-[10px] text-[#888888]">Container</span>
                                    <Select
                                        selectedKey={field.state.value || NONE_SENTINEL}
                                        onSelectionChange={(key) => field.handleChange(key === NONE_SENTINEL ? "" : String(key))}
                                        aria-label="Output container"
                                    >
                                        <SelectTrigger className="h-5 min-h-0 px-1.5 py-0 text-[10px] bg-[var(--surface-elevated)] border-[#2a2a3e] rounded-[3px] text-[#888888] w-[90px]">
                                            <SelectValue />
                                        </SelectTrigger>
                                        <SelectPopover>
                                            <SelectListBox>
                                                <SelectItem id={NONE_SENTINEL} textValue={codecInfo?.container?.toUpperCase() ?? "Auto"}>
                                                    {codecInfo?.container?.toUpperCase() ?? "Auto"} (default)
                                                </SelectItem>
                                                <SelectItem id="mp4" textValue="MP4">MP4</SelectItem>
                                                <SelectItem id="mkv" textValue="MKV">MKV</SelectItem>
                                                <SelectItem id="webm" textValue="WebM">WebM</SelectItem>
                                                <SelectItem id="mov" textValue="MOV">MOV</SelectItem>
                                                <SelectItem id="ts" textValue="TS">TS</SelectItem>
                                                <SelectItem id="avi" textValue="AVI">AVI</SelectItem>
                                            </SelectListBox>
                                        </SelectPopover>
                                    </Select>
                                </div>
                            )}
                        </form.Field>
                    )}

                    {/* Audio codec (visible when recode active) */}
                    {recodeActive && (
                        <form.Field name="recodeAudioMode">
                            {(field) => (
                                <div className="flex items-center justify-between pl-3">
                                    <span className="text-[10px] text-[#888888]">Audio</span>
                                    <Select
                                        selectedKey={field.state.value}
                                        onSelectionChange={(key) => field.handleChange(String(key))}
                                        aria-label="Audio handling"
                                    >
                                        <SelectTrigger className="h-5 min-h-0 px-1.5 py-0 text-[10px] bg-[var(--surface-elevated)] border-[#2a2a3e] rounded-[3px] text-[#888888] w-[90px]">
                                            <SelectValue />
                                        </SelectTrigger>
                                        <SelectPopover>
                                            <SelectListBox>
                                                <SelectItem id="copy" textValue="Copy">Copy</SelectItem>
                                                <SelectItem id="auto" textValue="Auto">Auto</SelectItem>
                                                {expertMode && audioCodecs.map((c) => (
                                                    c.encoders.map((enc) => (
                                                        <SelectItem key={enc.encoderName} id={enc.encoderName} textValue={enc.displayName}>
                                                            {enc.displayName}
                                                        </SelectItem>
                                                    ))
                                                ))}
                                            </SelectListBox>
                                        </SelectPopover>
                                    </Select>
                                </div>
                            )}
                        </form.Field>
                    )}

                    {/* Extract Audio */}
                    <form.Field name="extractAudio">
                        {(field) => (
                            <div className="flex items-center justify-between">
                                <span className="text-[11px] text-[#aaaaaa]">Extract Audio</span>
                                <Select
                                    selectedKey={field.state.value || NONE_SENTINEL}
                                    onSelectionChange={(key) => field.handleChange(key === NONE_SENTINEL ? "" : String(key))}
                                    aria-label="Extract audio"
                                >
                                    <SelectTrigger className="h-6 min-h-0 px-2 py-0 text-[11px] bg-[var(--surface-elevated)] border-[#2a2a3e] rounded-[4px] text-[#aaaaaa] w-[90px]">
                                        <SelectValue />
                                    </SelectTrigger>
                                    <SelectPopover>
                                        <SelectListBox>
                                            <SelectItem id={NONE_SENTINEL} textValue="Off">Off</SelectItem>
                                            <SelectItem id="mp3" textValue="MP3">MP3</SelectItem>
                                            <SelectItem id="aac" textValue="AAC">AAC</SelectItem>
                                            <SelectItem id="opus" textValue="Opus">Opus</SelectItem>
                                            <SelectItem id="flac" textValue="FLAC">FLAC</SelectItem>
                                        </SelectListBox>
                                    </SelectPopover>
                                </Select>
                            </div>
                        )}
                    </form.Field>
                </div>
            </section>

            {/* Embed Options */}
            <section className="p-3 border-b border-[#1a1a2e]">
                <h3 className="text-[10px] font-bold uppercase tracking-widest text-[#666666] mb-2">
                    Embed
                </h3>
                <div className="flex flex-col gap-1.5">
                    <form.Field name="embedThumbnail">
                        {(field) => (
                            <Checkbox
                                isSelected={field.state.value}
                                onChange={field.handleChange}
                                className="gap-2 text-[11px] text-[#aaaaaa]"
                            >
                                Thumbnail
                            </Checkbox>
                        )}
                    </form.Field>
                    <form.Field name="embedSubtitles">
                        {(field) => (
                            <Checkbox
                                isSelected={field.state.value}
                                onChange={field.handleChange}
                                className="gap-2 text-[11px] text-[#aaaaaa]"
                            >
                                Subtitles
                            </Checkbox>
                        )}
                    </form.Field>
                    <form.Field name="normalizeAudio">
                        {(field) => (
                            <Checkbox
                                isSelected={field.state.value}
                                onChange={field.handleChange}
                                className="gap-2 text-[11px] text-[#aaaaaa]"
                            >
                                Normalize Audio
                            </Checkbox>
                        )}
                    </form.Field>
                </div>
            </section>

            {/* Spacer */}
            <div className="flex-1" />

            {/* Download actions */}
            <section className="p-3 border-t border-[#1a1a2e]">
                <Button
                    variant="ghost"
                    type="submit"
                    isDisabled={!hasSelection || isSubmitting}
                    className={cn(
                        "w-full flex items-center justify-center gap-2 py-2 rounded-[6px] text-[13px] font-medium transition-colors",
                        hasSelection && !isSubmitting
                            ? "bg-[#4a9eff] text-white hover:bg-[#3a8ef0] data-[hovered]:bg-[#3a8ef0]"
                            : "bg-[#1a2a4a] text-[#444444] cursor-not-allowed",
                    )}
                >
                    <Download className="w-4 h-4" />
                    {isSubmitting ? "Starting…" : "Download"}
                </Button>
            </section>
        </form>
    );
}
