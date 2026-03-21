// Collapsible download options section (Zone 3) for the format dialog.
//
// Contains save directory, remux, audio extraction, subtitles, and thumbnail controls.

import { cn } from "@/lib/utils";
import { useState } from "react";
import { ChevronUp, ChevronDown, FolderOpen, Settings2, AlertTriangle } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
    Collapsible,
    CollapsibleContent,
    CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "@/components/ui/popover";
import {
    Select,
    SelectContent,
    SelectGroup,
    SelectItem,
    SelectLabel,
    SelectSeparator,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { optionsSummary } from "./utils/tableHelpers";
import { getNormSelectValue, handleNormSelectChange } from "./utils/normalization";
import { NormalizationCustomControls } from "./NormalizationCustomControls";
import { codecsQueryOptions } from "../api/codecs";
import { audioCodecsQueryOptions } from "../api/audioCodecs";
import type {
    AppSettings,
    AudioCodecInfo,
    AudioFormat,
    ContainerFormat,
    DownloadOptions,
    RecodeAudioMode,
    VideoCodecInfo,
} from "../types";

// -- Constants --------------------------------------------------------

const REMUX_OPTIONS: Array<{ value: ContainerFormat; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
];

/** Recode container options (includes Auto which maps to null). */
const RECODE_CONTAINER_OPTIONS: Array<{ value: string; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
    { value: "mov", label: "MOV" },
    { value: "ogg", label: "OGG" },
    { value: "ts", label: "TS" },
    { value: "avi", label: "AVI" },
];

/** Codec-level value prefix used in the video Recode Select when Expert Mode is off. */
const CODEC_VALUE_PREFIX = "codec:";

/** Encoder-level value prefix used in the video Recode Select when Expert Mode is on. */
const ENCODER_VALUE_PREFIX = "encoder:";

/** Prefix for audio codec selection (default mode). */
const AUDIO_CODEC_PREFIX = "audio-codec:";

/** Prefix for audio encoder selection (expert mode). */
const AUDIO_ENCODER_PREFIX = "audio-encoder:";

/** Sentinel value for "copy" audio (stream copy unchanged). */
const AUDIO_COPY_VALUE = "audio-copy";

const AUDIO_OPTIONS: Array<{ value: AudioFormat; label: string }> = [
    { value: "mp3", label: "MP3" },
    { value: "aac", label: "AAC" },
    { value: "opus", label: "Opus" },
    { value: "flac", label: "FLAC" },
];

const NONE_SENTINEL = "none";

// -- Helpers ----------------------------------------------------------------

/**
 * Derive the current Select value for the video Recode dropdown from options state.
 * Returns "none", "codec:<name>", or "encoder:<name>" depending on expert mode.
 */
function getRecodeSelectValue(
    options: DownloadOptions,
    codecs: VideoCodecInfo[],
    expertMode: boolean,
): string {
    if (!options.recodeVideo) return NONE_SENTINEL;
    if (!expertMode) return `${CODEC_VALUE_PREFIX}${options.recodeVideo}`;
    // In expert mode: if videoEncoder is set return encoder value, else fall back to codec value
    if (options.videoEncoder) return `${ENCODER_VALUE_PREFIX}${options.videoEncoder}`;
    // Try to find a matching codec by the recodeVideo container field used as codec key
    const matchingCodec = codecs.find((c) => c.codec === options.recodeVideo);
    if (matchingCodec && matchingCodec.encoders.length > 0) {
        return `${ENCODER_VALUE_PREFIX}${matchingCodec.encoders[0].encoderName}`;
    }
    return `${CODEC_VALUE_PREFIX}${options.recodeVideo}`;
}

/**
 * Derive the current Select value for the audio recode dropdown.
 * Returns AUDIO_COPY_VALUE, "audio-codec:<name>", or "audio-encoder:<name>" depending on mode/expert.
 */
function getAudioRecodeSelectValue(
    recodeAudio: RecodeAudioMode | null,
    audioExpert: boolean,
): string {
    if (!recodeAudio || recodeAudio.mode === "copy") return AUDIO_COPY_VALUE;
    if (recodeAudio.mode === "auto") {
        // auto is used in default mode; show as codec value if we can derive it
        return AUDIO_COPY_VALUE; // fallback — auto without a named codec shows as Copy
    }
    if (recodeAudio.mode === "encoder") {
        if (audioExpert) return `${AUDIO_ENCODER_PREFIX}${recodeAudio.name}`;
        // Non-expert: show as copy since we can't easily reverse-map encoder→codec here
        return AUDIO_COPY_VALUE;
    }
    return AUDIO_COPY_VALUE;
}

/**
 * Check if the current audio selection is compatible with the given container.
 * Returns null when compatible (or Copy is selected), or an incompatibility message.
 */
function getAudioCompatibilityWarning(
    recodeAudio: RecodeAudioMode | null,
    container: string | null,
    audioCodecs: AudioCodecInfo[],
): string | null {
    if (!container) return null;
    if (!recodeAudio || recodeAudio.mode === "copy") return null;

    let codecName: string | null = null;
    if (recodeAudio.mode === "encoder") {
        // Find the parent codec for this encoder
        const parentCodec = audioCodecs.find((c) =>
            c.encoders.some((e) => e.encoderName === recodeAudio.name),
        );
        codecName = parentCodec?.codec ?? null;
    }

    if (!codecName) return null;

    const codecInfo = audioCodecs.find((c) => c.codec === codecName);
    if (!codecInfo) return null;

    if (!codecInfo.supportedContainers.includes(container)) {
        const containerLabel = container.toUpperCase();
        return `${codecInfo.displayName} is not compatible with ${containerLabel}. Audio reset to Copy.`;
    }

    return null;
}

// -- Types ----------------------------------------------------------------

interface DownloadOptionsPanelProps {
    options: DownloadOptions;
    setOptions: React.Dispatch<React.SetStateAction<DownloadOptions>>;
    settings: AppSettings | null;
    subtitleLangs: string[];
    showOptions: boolean;
    setShowOptions: (open: boolean) => void;
    onBrowseDir: () => void;
    onSubLangSelect: (lang: string) => void;
}

/** Zone 3: Collapsible download options with save directory, remux, audio, subtitles, thumbnail. */
export function DownloadOptionsPanel({
    options,
    setOptions,
    settings,
    subtitleLangs,
    showOptions,
    setShowOptions,
    onBrowseDir,
    onSubLangSelect,
}: DownloadOptionsPanelProps) {
    const { data: codecs = [] } = useQuery(codecsQueryOptions());
    const [videoExpert, setVideoExpert] = useState(false);
    const [audioExpert, setAudioExpert] = useState(false);
    const [audioCompatWarning, setAudioCompatWarning] = useState<string | null>(null);

    // Only include video codecs that have at least one available encoder
    const availableCodecs = codecs.filter((c) => c.encoders.length > 0);

    const recodeActive = !!options.recodeVideo;

    // Fetch audio codecs filtered by current recode container
    const { data: audioCodecs = [] } = useQuery({
        ...audioCodecsQueryOptions(options.recodeContainer),
        enabled: recodeActive,
    });

    const recodeSelectValue = getRecodeSelectValue(options, availableCodecs, videoExpert);
    const audioSelectValue = getAudioRecodeSelectValue(options.recodeAudio, audioExpert);

    const handleRecodeChange = (val: string) => {
        setOptions((prev) => {
            if (val === NONE_SENTINEL) {
                return {
                    ...prev,
                    recodeVideo: null,
                    videoEncoder: null,
                    recodeContainer: null,
                    recodeAudio: null,
                    remux: prev.remux,
                };
            }
            if (val.startsWith(CODEC_VALUE_PREFIX)) {
                const codec = val.slice(CODEC_VALUE_PREFIX.length);
                const codecInfo = availableCodecs.find((c) => c.codec === codec);
                const container = (codecInfo?.defaultContainer ?? "mp4") as ContainerFormat;
                return { ...prev, recodeVideo: container, videoEncoder: null, remux: null };
            }
            if (val.startsWith(ENCODER_VALUE_PREFIX)) {
                const encoderName = val.slice(ENCODER_VALUE_PREFIX.length);
                const parentCodec = availableCodecs.find((c) =>
                    c.encoders.some((e) => e.encoderName === encoderName),
                );
                const container = (parentCodec?.defaultContainer ?? "mp4") as ContainerFormat;
                return { ...prev, recodeVideo: container, videoEncoder: encoderName, remux: null };
            }
            return prev;
        });
        // Clear compat warning when recode is deactivated
        if (val === NONE_SENTINEL) setAudioCompatWarning(null);
    };

    const handleContainerChange = (val: string) => {
        const newContainer = val === NONE_SENTINEL ? null : val;
        setOptions((prev) => {
            const newOptions = { ...prev, recodeContainer: newContainer };

            // Check if current audio selection is still compatible
            if (prev.recodeAudio && prev.recodeAudio.mode !== "copy" && newContainer) {
                const warning = getAudioCompatibilityWarning(
                    prev.recodeAudio,
                    newContainer,
                    audioCodecs,
                );
                if (warning) {
                    setAudioCompatWarning(warning);
                    return { ...newOptions, recodeAudio: { mode: "copy" } };
                }
            }

            setAudioCompatWarning(null);
            return newOptions;
        });
    };

    const handleAudioRecodeChange = (val: string) => {
        setAudioCompatWarning(null);
        setOptions((prev) => {
            if (val === AUDIO_COPY_VALUE) {
                return { ...prev, recodeAudio: { mode: "copy" } };
            }
            if (val.startsWith(AUDIO_CODEC_PREFIX)) {
                // Default mode: codec selected → auto-pick best encoder
                return { ...prev, recodeAudio: { mode: "auto" } };
            }
            if (val.startsWith(AUDIO_ENCODER_PREFIX)) {
                const encoderName = val.slice(AUDIO_ENCODER_PREFIX.length);
                return { ...prev, recodeAudio: { mode: "encoder", name: encoderName } };
            }
            return prev;
        });
    };

    return (
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
                        <Label className="options-label">
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
                                onClick={onBrowseDir}
                            >
                                <FolderOpen className="size-3" />
                            </Button>
                        </div>

                        {/* Remux */}
                        <Label className="options-label">
                            Remux
                        </Label>
                        <div className="flex flex-col gap-0.5">
                            <Select
                                value={options.remux ?? NONE_SENTINEL}
                                onValueChange={(val) => setOptions((prev) => {
                                    const remux = val === NONE_SENTINEL ? null : (val as ContainerFormat);
                                    return {
                                        ...prev,
                                        remux,
                                        recodeVideo: remux !== null ? null : prev.recodeVideo,
                                    };
                                })}
                            >
                                <SelectTrigger className={cn("h-7 text-xs", options.remux && "select-active")}>
                                    <SelectValue />
                                </SelectTrigger>
                                <SelectContent>
                                    <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                                    {REMUX_OPTIONS.map((o) => (
                                        <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                    ))}
                                </SelectContent>
                            </Select>
                            <p className="text-[10px] text-muted-foreground">
                                Copy streams — no quality loss.
                            </p>
                        </div>

                        {/* Recode Video */}
                        <Label className="options-label">
                            Recode
                        </Label>
                        <div className="flex flex-col gap-0.5">
                            <Select
                                value={recodeSelectValue}
                                onValueChange={handleRecodeChange}
                            >
                                <SelectTrigger className={cn("h-7 text-xs", options.recodeVideo && "select-active")}>
                                    <SelectValue />
                                </SelectTrigger>
                                <SelectContent>
                                    <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                                    {!videoExpert ? (
                                        // Default mode: one entry per codec (display name only)
                                        availableCodecs.map((codec) => (
                                            <SelectItem
                                                key={codec.codec}
                                                value={`${CODEC_VALUE_PREFIX}${codec.codec}`}
                                            >
                                                {codec.displayName}
                                            </SelectItem>
                                        ))
                                    ) : (
                                        // Expert mode: entries grouped by codec, showing encoder names
                                        availableCodecs.map((codec, idx) => (
                                            <SelectGroup key={codec.codec}>
                                                {idx > 0 && <SelectSeparator />}
                                                <SelectLabel>{codec.displayName}</SelectLabel>
                                                {codec.encoders.map((enc) => (
                                                    <SelectItem
                                                        key={enc.encoderName}
                                                        value={`${ENCODER_VALUE_PREFIX}${enc.encoderName}`}
                                                    >
                                                        {enc.encoderName}
                                                    </SelectItem>
                                                ))}
                                            </SelectGroup>
                                        ))
                                    )}
                                </SelectContent>
                            </Select>
                            <div className="flex items-center justify-between">
                                <p className="text-[10px] text-muted-foreground">
                                    Re-encode video — use when remux fails.
                                </p>
                                <label className="flex items-center gap-1 text-[10px] text-muted-foreground cursor-pointer select-none">
                                    <input
                                        type="checkbox"
                                        className="size-2.5 accent-primary"
                                        checked={videoExpert}
                                        onChange={(e) => {
                                            setVideoExpert(e.target.checked);
                                            // Switching off video expert mode clears the encoder override
                                            if (!e.target.checked) {
                                                setOptions((prev) => ({ ...prev, videoEncoder: null }));
                                            }
                                        }}
                                    />
                                    Expert
                                </label>
                            </div>
                        </div>

                        {/* Recode sub-options: Container + Audio (only when Recode is active) */}
                        {recodeActive && (
                            <>
                                {/* Container */}
                                <Label className="options-label pl-3 text-muted-foreground/70">
                                    Container
                                </Label>
                                <Select
                                    value={options.recodeContainer ?? NONE_SENTINEL}
                                    onValueChange={handleContainerChange}
                                >
                                    <SelectTrigger className={cn("h-7 text-xs", options.recodeContainer && "select-active")}>
                                        <SelectValue placeholder="Auto" />
                                    </SelectTrigger>
                                    <SelectContent>
                                        <SelectItem value={NONE_SENTINEL}>Auto</SelectItem>
                                        {RECODE_CONTAINER_OPTIONS.map((o) => (
                                            <SelectItem key={o.value} value={o.value}>
                                                {o.label}
                                            </SelectItem>
                                        ))}
                                    </SelectContent>
                                </Select>

                                {/* Audio recode */}
                                <Label className="options-label pl-3 text-muted-foreground/70 self-start pt-1">
                                    Audio
                                </Label>
                                <div className="flex flex-col gap-0.5">
                                    <Select
                                        value={audioSelectValue}
                                        onValueChange={handleAudioRecodeChange}
                                    >
                                        <SelectTrigger className={cn("h-7 text-xs", options.recodeAudio && options.recodeAudio.mode !== "copy" && "select-active")}>
                                            <SelectValue />
                                        </SelectTrigger>
                                        <SelectContent>
                                            <SelectItem value={AUDIO_COPY_VALUE}>Copy</SelectItem>
                                            {!audioExpert ? (
                                                // Default mode: codec display names
                                                audioCodecs.map((codec) => (
                                                    <SelectItem
                                                        key={codec.codec}
                                                        value={`${AUDIO_CODEC_PREFIX}${codec.codec}`}
                                                    >
                                                        {codec.displayName}
                                                    </SelectItem>
                                                ))
                                            ) : (
                                                // Expert mode: encoder names grouped by codec
                                                audioCodecs.map((codec, idx) => (
                                                    <SelectGroup key={codec.codec}>
                                                        {idx > 0 && <SelectSeparator />}
                                                        <SelectLabel>{codec.displayName}</SelectLabel>
                                                        {codec.encoders.map((enc) => (
                                                            <SelectItem
                                                                key={enc.encoderName}
                                                                value={`${AUDIO_ENCODER_PREFIX}${enc.encoderName}`}
                                                            >
                                                                {enc.encoderName}
                                                            </SelectItem>
                                                        ))}
                                                    </SelectGroup>
                                                ))
                                            )}
                                        </SelectContent>
                                    </Select>
                                    <div className="flex items-center justify-end">
                                        <label className="flex items-center gap-1 text-[10px] text-muted-foreground cursor-pointer select-none">
                                            <input
                                                type="checkbox"
                                                className="size-2.5 accent-primary"
                                                checked={audioExpert}
                                                onChange={(e) => {
                                                    setAudioExpert(e.target.checked);
                                                    // Switching off audio expert mode resets to Copy
                                                    if (!e.target.checked) {
                                                        setOptions((prev) => ({
                                                            ...prev,
                                                            recodeAudio: { mode: "copy" },
                                                        }));
                                                    }
                                                }}
                                            />
                                            Expert
                                        </label>
                                    </div>
                                    {audioCompatWarning && (
                                        <Alert className="mt-1 py-2 px-3 col-span-2">
                                            <AlertTriangle className="size-3.5" />
                                            <AlertDescription className="text-[10px]">
                                                {audioCompatWarning}
                                            </AlertDescription>
                                        </Alert>
                                    )}
                                </div>
                            </>
                        )}

                        {/* Extract Audio */}
                        <Label className="options-label">
                            Audio
                        </Label>
                        <Select
                            value={options.extractAudio ?? NONE_SENTINEL}
                            onValueChange={(val) => setOptions((prev) => ({
                                ...prev,
                                extractAudio: val === NONE_SENTINEL ? null : (val as AudioFormat),
                            }))}
                        >
                            <SelectTrigger className={cn("h-7 text-xs", options.extractAudio && "select-active")}>
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
                        <Label className="options-label">
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
                                                                onCheckedChange={() => onSubLangSelect(lang)}
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
                        <Label className="options-label">
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

                        {/* Audio Normalization */}
                        <Label className="options-label self-start pt-1.5">
                            Normalize
                        </Label>
                        <div className="flex flex-col gap-0">
                            <Select
                                value={getNormSelectValue(options)}
                                onValueChange={(val) =>
                                    setOptions((prev) => handleNormSelectChange(prev, val))
                                }
                            >
                                <SelectTrigger className={cn("h-7 text-xs", getNormSelectValue(options) !== "default" && "select-active")}>
                                    <SelectValue />
                                </SelectTrigger>
                                <SelectContent>
                                    <SelectItem value="default">Use Settings Default</SelectItem>
                                    <SelectItem value="off">Off</SelectItem>
                                    <SelectItem value="peak">Peak</SelectItem>
                                    <SelectItem value="loudnorm-streaming">Loudnorm (Streaming -14 LUFS)</SelectItem>
                                    <SelectItem value="loudnorm-broadcast">Loudnorm (Broadcast -23 LUFS)</SelectItem>
                                    <SelectItem value="loudnorm-loud">Loudnorm (Loud -11 LUFS)</SelectItem>
                                    <SelectItem value="custom">Custom...</SelectItem>
                                </SelectContent>
                            </Select>
                            {getNormSelectValue(options) === "custom" && (
                                <NormalizationCustomControls
                                    value={options}
                                    onChange={(next) => setOptions(next)}
                                    idPrefix="dop-norm"
                                />
                            )}
                        </div>
                    </div>
                </CollapsibleContent>
            </Collapsible>
        </div>
    );
}
