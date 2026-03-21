// Collapsible download options section (Zone 3) for the format dialog.
//
// Contains save directory, remux, audio extraction, subtitles, and thumbnail controls.

import { cn } from "@/lib/utils";
import { useState } from "react";
import { ChevronUp, ChevronDown, FolderOpen, Settings2 } from "lucide-react";
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
import { optionsSummary } from "./utils/tableHelpers";
import { getNormSelectValue, handleNormSelectChange } from "./utils/normalization";
import { NormalizationCustomControls } from "./NormalizationCustomControls";
import { codecsQueryOptions } from "../api/codecs";
import type {
    AppSettings,
    AudioFormat,
    ContainerFormat,
    DownloadOptions,
    VideoCodecInfo,
} from "../types";

// -- Constants --------------------------------------------------------

const REMUX_OPTIONS: Array<{ value: ContainerFormat; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
];

/** Codec-level value prefix used in the Recode Select when Expert Mode is off. */
const CODEC_VALUE_PREFIX = "codec:";

/** Encoder-level value prefix used in the Recode Select when Expert Mode is on. */
const ENCODER_VALUE_PREFIX = "encoder:";

const AUDIO_OPTIONS: Array<{ value: AudioFormat; label: string }> = [
    { value: "mp3", label: "MP3" },
    { value: "aac", label: "AAC" },
    { value: "opus", label: "Opus" },
    { value: "flac", label: "FLAC" },
];

const NONE_SENTINEL = "none";

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

/**
 * Derive the current Select value for the Recode dropdown from options state.
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
    const [expertMode, setExpertMode] = useState(false);

    // Only include codecs that have at least one available encoder
    const availableCodecs = codecs.filter((c) => c.encoders.length > 0);

    const recodeSelectValue = getRecodeSelectValue(options, availableCodecs, expertMode);

    const handleRecodeChange = (val: string) => {
        setOptions((prev) => {
            if (val === NONE_SENTINEL) {
                return { ...prev, recodeVideo: null, videoEncoder: null, remux: prev.remux };
            }
            if (val.startsWith(CODEC_VALUE_PREFIX)) {
                // Codec-level selection: use backend-provided default container
                const codec = val.slice(CODEC_VALUE_PREFIX.length);
                const codecInfo = availableCodecs.find((c) => c.codec === codec);
                const container = (codecInfo?.defaultContainer ?? "mp4") as ContainerFormat;
                return { ...prev, recodeVideo: container, videoEncoder: null, remux: null };
            }
            if (val.startsWith(ENCODER_VALUE_PREFIX)) {
                // Encoder-level selection: use parent codec's default container
                const encoderName = val.slice(ENCODER_VALUE_PREFIX.length);
                const parentCodec = availableCodecs.find((c) =>
                    c.encoders.some((e) => e.encoderName === encoderName)
                );
                const container = (parentCodec?.defaultContainer ?? "mp4") as ContainerFormat;
                return { ...prev, recodeVideo: container, videoEncoder: encoderName, remux: null };
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
                                    {!expertMode ? (
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
                                        checked={expertMode}
                                        onChange={(e) => {
                                            setExpertMode(e.target.checked);
                                            // Switching off expert mode clears the encoder override
                                            if (!e.target.checked) {
                                                setOptions((prev) => ({ ...prev, videoEncoder: null }));
                                            }
                                        }}
                                    />
                                    Expert
                                </label>
                            </div>
                        </div>

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
