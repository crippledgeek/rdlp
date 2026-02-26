import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import {
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "@/components/ui/popover";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import type {
    AppSettings,
    AudioFormat,
    ContainerFormat,
    DownloadOptions,
} from "../types";
import { getNormSelectValue, handleNormSelectChange } from "./utils/normalization";
import { NormalizationCustomControls } from "./NormalizationCustomControls";

/** A format preset with a human-readable label and yt-dlp selector string. */
export interface FormatPreset {
    id: string;
    label: string;
    selector: string;
}

/** Shared preset definitions used by both the popover and FormatDialog. */
export const PRESETS: FormatPreset[] = [
    { id: "best", label: "Best Quality", selector: "bestvideo+bestaudio/best" },
    {
        id: "1080p",
        label: "1080p",
        selector: "bestvideo[height<=1080]+bestaudio/best[height<=1080]",
    },
    {
        id: "720p",
        label: "720p",
        selector: "bestvideo[height<=720]+bestaudio/best[height<=720]",
    },
    { id: "audio-only", label: "Audio Only", selector: "bestaudio/best" },
];

/** Common remux options shown in the UI. */
const REMUX_OPTIONS: Array<{ value: ContainerFormat; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
];

/** Common audio extraction options shown in the UI. */
const AUDIO_OPTIONS: Array<{ value: AudioFormat; label: string }> = [
    { value: "mp3", label: "MP3" },
    { value: "aac", label: "AAC" },
    { value: "opus", label: "Opus" },
    { value: "flac", label: "FLAC" },
];

/** Sentinel value for "no selection" in Radix Select (empty string is not supported). */
const NONE_SENTINEL = "none";

/**
 * Build a `DownloadOptions` from persisted settings.
 *
 * Leaves `format` as `null` so the backend applies its smart default
 * selector (`bv*+ba/b` with FFmpeg, `b/bv+ba` without).
 */
export function buildDefaultOptions(
    settings: AppSettings | null,
): DownloadOptions {
    return {
        format: null,
        outputDir: settings?.output_dir ?? null,
        subtitles: (settings?.default_subtitle_langs ?? []).length > 0,
        subtitleLangs: settings?.default_subtitle_langs ?? [],
        remux: settings?.default_remux ?? null,
        extractAudio: settings?.default_extract_audio ?? null,
        embedThumbnail: settings?.embed_thumbnail ?? true,
        audioMultistreams: false,
        recodeVideo: null,
        normalizeAudio: settings?.normalize_audio ?? false,
        loudnorm: settings?.loudnorm ?? false,
        loudnormPreset: settings?.loudnorm_preset ?? null,
        loudnormTargetI: settings?.loudnorm_target_i ?? null,
        loudnormTargetTp: settings?.loudnorm_target_tp ?? null,
        loudnormTargetLra: settings?.loudnorm_target_lra ?? null,
        loudnormDynamic: settings?.loudnorm_dynamic ?? false,
        loudnormPrecompress: settings?.loudnorm_precompress ?? false,
        normalizeBoost: settings?.normalize_boost ?? false,
        normalizeBoostDb: settings?.normalize_boost_db ?? null,
    };
}

interface FormatOptionsPanelProps {
    value: DownloadOptions;
    onChange: (next: DownloadOptions) => void;
    availableSubtitleLangs?: string[];
    /** Hide presets (e.g. when the FormatDialog manages format selection). */
    hidePresets?: boolean;
}

/** Derive which preset ID matches the current format selector, if any. */
function activePreset(format: string | null): string | null {
    if (!format) return null;
    return PRESETS.find((p) => p.selector === format)?.id ?? null;
}

/** Controlled panel for download options: presets, remux, audio, subs, thumbnail. */
export function FormatOptionsPanel({
    value,
    onChange,
    availableSubtitleLangs = [],
    hidePresets = false,
}: FormatOptionsPanelProps) {
    const currentPreset = activePreset(value.format);

    const handlePreset = (presetId: string) => {
        const preset = PRESETS.find((p) => p.id === presetId);
        if (preset) onChange({ ...value, format: preset.selector });
    };

    const handleRemux = (val: string) => {
        onChange({
            ...value,
            remux: val === NONE_SENTINEL ? null : (val as ContainerFormat),
        });
    };

    const handleAudio = (val: string) => {
        onChange({
            ...value,
            extractAudio:
                val === NONE_SENTINEL ? null : (val as AudioFormat),
        });
    };

    const handleSubtitles = (checked: boolean) => {
        onChange({ ...value, subtitles: checked });
    };

    const handleSubLangs = (raw: string) => {
        const langs = raw
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean);
        onChange({ ...value, subtitleLangs: langs });
    };

    const handleSubLangSelect = (val: string) => {
        if (val === NONE_SENTINEL) {
            onChange({ ...value, subtitleLangs: [] });
            return;
        }
        const current = value.subtitleLangs;
        const next = current.includes(val)
            ? current.filter((l) => l !== val)
            : [...current, val];
        onChange({ ...value, subtitleLangs: next });
    };

    return (
        <div className="flex flex-col gap-1.5">
            {!hidePresets && (
                <div className="flex flex-col gap-1">
                    <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                        Quality Preset
                    </Label>
                    <RadioGroup
                        value={currentPreset ?? ""}
                        onValueChange={handlePreset}
                        className="flex flex-wrap gap-1"
                    >
                        {PRESETS.map((p) => (
                            <Label
                                key={p.id}
                                htmlFor={`preset-${p.id}`}
                                className={cn(
                                    "inline-flex items-center gap-[5px] px-2.5 py-[5px] border border-white/[0.06] rounded-sm bg-card text-xs text-muted-foreground cursor-pointer transition-colors",
                                    "hover:border-white/[0.12] hover:text-foreground",
                                    currentPreset === p.id &&
                                        "border-primary text-primary bg-primary/[0.12]",
                                )}
                            >
                                <RadioGroupItem
                                    value={p.id}
                                    id={`preset-${p.id}`}
                                    className="size-3.5"
                                />
                                {p.label}
                            </Label>
                        ))}
                    </RadioGroup>
                </div>
            )}

            <div className="flex flex-col gap-1">
                <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                    Remux
                </Label>
                <Select
                    value={value.remux ?? NONE_SENTINEL}
                    onValueChange={handleRemux}
                >
                    <SelectTrigger size="sm" className={cn("w-full text-xs", value.remux && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                        {REMUX_OPTIONS.map((o) => (
                            <SelectItem key={o.value} value={o.value}>
                                {o.label}
                            </SelectItem>
                        ))}
                    </SelectContent>
                </Select>
            </div>

            <div className="flex flex-col gap-1">
                <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                    Extract Audio
                </Label>
                <Select
                    value={value.extractAudio ?? NONE_SENTINEL}
                    onValueChange={handleAudio}
                >
                    <SelectTrigger size="sm" className={cn("w-full text-xs", value.extractAudio && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                        {AUDIO_OPTIONS.map((o) => (
                            <SelectItem key={o.value} value={o.value}>
                                {o.label}
                            </SelectItem>
                        ))}
                    </SelectContent>
                </Select>
            </div>

            <div className="flex flex-col gap-1">
                <div className="flex items-center gap-2">
                    <Checkbox
                        checked={value.subtitles}
                        onCheckedChange={(checked) =>
                            handleSubtitles(checked === true)
                        }
                        id="subtitles"
                    />
                    <Label
                        htmlFor="subtitles"
                        className="text-[11px] font-semibold text-muted-foreground tracking-wide cursor-pointer"
                    >
                        Download Subtitles
                    </Label>
                </div>
                {value.subtitles && (
                    <div className="mt-1.5">
                        {availableSubtitleLangs.length > 0 ? (
                            <Popover>
                                <PopoverTrigger asChild>
                                    <Button
                                        variant="outline"
                                        size="sm"
                                        className="w-full justify-between text-xs font-normal"
                                    >
                                        <span className="truncate">
                                            {value.subtitleLangs.length > 0
                                                ? value.subtitleLangs.join(", ")
                                                : "None"}
                                        </span>
                                    </Button>
                                </PopoverTrigger>
                                <PopoverContent
                                    align="start"
                                    className="w-(--radix-popover-trigger-width) p-2"
                                >
                                    <div className="flex flex-col gap-1 max-h-48 overflow-y-auto">
                                        {availableSubtitleLangs.map((lang) => (
                                            <Label
                                                key={lang}
                                                htmlFor={`sub-lang-${lang}`}
                                                className="flex items-center gap-2 rounded-sm px-2 py-1.5 text-xs cursor-pointer hover:bg-accent"
                                            >
                                                <Checkbox
                                                    id={`sub-lang-${lang}`}
                                                    checked={value.subtitleLangs.includes(lang)}
                                                    onCheckedChange={() =>
                                                        handleSubLangSelect(lang)
                                                    }
                                                />
                                                {lang}
                                            </Label>
                                        ))}
                                    </div>
                                </PopoverContent>
                            </Popover>
                        ) : (
                            <Input
                                className="font-mono text-xs"
                                type="text"
                                placeholder="en,sv,ja"
                                value={value.subtitleLangs.join(",")}
                                onChange={(e) => handleSubLangs(e.target.value)}
                            />
                        )}
                    </div>
                )}
            </div>

            <div className="flex flex-col gap-1">
                <div className="flex items-center gap-2">
                    <Checkbox
                        checked={value.embedThumbnail}
                        onCheckedChange={(checked) =>
                            onChange({
                                ...value,
                                embedThumbnail: checked === true,
                            })
                        }
                        id="embed-thumbnail"
                    />
                    <Label
                        htmlFor="embed-thumbnail"
                        className="text-[11px] font-semibold text-muted-foreground tracking-wide cursor-pointer"
                    >
                        Embed Thumbnail
                    </Label>
                </div>
            </div>

            <div className="flex flex-col gap-1">
                <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                    Audio Normalization
                </Label>
                <Select
                    value={getNormSelectValue(value)}
                    onValueChange={(val) => onChange(handleNormSelectChange(value, val))}
                >
                    <SelectTrigger size="sm" className={cn("w-full text-xs", getNormSelectValue(value) !== "default" && "select-active")}>
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
                {getNormSelectValue(value) === "custom" && (
                    <NormalizationCustomControls
                        value={value}
                        onChange={onChange}
                        idPrefix="fop-norm"
                    />
                )}
            </div>
        </div>
    );
}
