import { cn } from "@/lib/utils";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import type { AudioFormat, ContainerFormat, DownloadOptions } from "../types";

/** A format preset with a human-readable label and yt-dlp selector string. */
export interface FormatPreset {
    id: string;
    label: string;
    selector: string;
}

/** Shared preset definitions used by both the popover and FormatDialog. */
export const PRESETS: FormatPreset[] = [
    { id: "best", label: "Best Quality", selector: "bestvideo+bestaudio/best" },
    { id: "1080p", label: "1080p", selector: "bestvideo[height<=1080]+bestaudio/best[height<=1080]" },
    { id: "720p", label: "720p", selector: "bestvideo[height<=720]+bestaudio/best[height<=720]" },
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

/**
 * Tailwind classes for native `<select>` elements with a custom chevron icon.
 * Uses a data URI SVG background image to avoid inline style attributes.
 */
const selectClasses = cn(
    "h-8 rounded-md border border-input bg-card px-2.5 pr-7 text-xs font-medium text-muted-foreground",
    "cursor-pointer appearance-none transition-colors",
    "bg-[url('data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%2210%22%20height%3D%2210%22%20viewBox%3D%220%200%2024%2024%22%20fill%3D%22none%22%20stroke%3D%22%236b7280%22%20stroke-width%3D%222%22%20stroke-linecap%3D%22round%22%20stroke-linejoin%3D%22round%22%3E%3Cpolyline%20points%3D%226%209%2012%2015%2018%209%22%2F%3E%3C%2Fsvg%3E')] bg-no-repeat bg-[position:right_8px_center]",
    "focus:outline-none focus:ring-1 focus:ring-ring",
);

interface FormatOptionsPanelProps {
    value: DownloadOptions;
    onChange: (next: DownloadOptions) => void;
    availableSubtitleLangs?: string[];
    /** Hide presets (e.g. when the FormatDialog manages format selection). */
    hidePresets?: boolean;
}

/** Derive which preset ID matches the current format selector, if any. */
function activePreset(format: string | null): string | null {
    if (!format) return "best";
    const match = PRESETS.find((p) => p.selector === format);
    return match?.id ?? null;
}

/** Controlled panel for download options: presets, remux, audio, subs, thumbnail. */
export function FormatOptionsPanel({
    value,
    onChange,
    availableSubtitleLangs = [],
    hidePresets = false,
}: FormatOptionsPanelProps) {
    const currentPreset = activePreset(value.format);

    const handlePreset = (preset: FormatPreset) => {
        onChange({ ...value, format: preset.selector });
    };

    const handleRemux = (val: string) => {
        onChange({
            ...value,
            remux: val === "" ? null : (val as ContainerFormat),
        });
    };

    const handleAudio = (val: string) => {
        onChange({
            ...value,
            extractAudio: val === "" ? null : (val as AudioFormat),
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

    return (
        <div className="flex flex-col gap-1.5">
            {!hidePresets && (
                <div className="flex flex-col gap-1">
                    <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                        Quality Preset
                    </Label>
                    <div className="flex flex-wrap gap-1">
                        {PRESETS.map((p) => (
                            <label
                                key={p.id}
                                className={cn(
                                    "inline-flex items-center gap-[5px] px-2.5 py-[5px] border border-white/[0.06] rounded-sm bg-card text-xs text-muted-foreground cursor-pointer transition-colors",
                                    "hover:border-white/[0.12] hover:text-foreground",
                                    currentPreset === p.id && "border-primary text-primary bg-primary/[0.12]",
                                )}
                            >
                                <input
                                    type="radio"
                                    name="preset"
                                    checked={currentPreset === p.id}
                                    onChange={() => handlePreset(p)}
                                    className={cn(
                                        "appearance-none w-3 h-3 border-2 border-muted rounded-full bg-muted cursor-pointer shrink-0 relative transition-colors",
                                        currentPreset === p.id && "border-primary bg-primary after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:w-1 after:h-1 after:rounded-full after:bg-background",
                                    )}
                                />
                                {p.label}
                            </label>
                        ))}
                    </div>
                </div>
            )}

            <div className="flex flex-col gap-1">
                <Label
                    htmlFor="remux-select"
                    className="text-[11px] font-semibold text-muted-foreground tracking-wide"
                >
                    Remux
                </Label>
                <select
                    id="remux-select"
                    className={selectClasses}
                    value={value.remux ?? ""}
                    onChange={(e) => handleRemux(e.target.value)}
                >
                    <option value="">None</option>
                    {REMUX_OPTIONS.map((o) => (
                        <option key={o.value} value={o.value}>
                            {o.label}
                        </option>
                    ))}
                </select>
            </div>

            <div className="flex flex-col gap-1">
                <Label
                    htmlFor="audio-select"
                    className="text-[11px] font-semibold text-muted-foreground tracking-wide"
                >
                    Extract Audio
                </Label>
                <select
                    id="audio-select"
                    className={selectClasses}
                    value={value.extractAudio ?? ""}
                    onChange={(e) => handleAudio(e.target.value)}
                >
                    <option value="">None</option>
                    {AUDIO_OPTIONS.map((o) => (
                        <option key={o.value} value={o.value}>
                            {o.label}
                        </option>
                    ))}
                </select>
            </div>

            <div className="flex flex-col gap-1">
                <div className="flex items-center gap-2">
                    <Checkbox
                        checked={value.subtitles}
                        onCheckedChange={(checked) => handleSubtitles(checked === true)}
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
                            <select
                                className={cn(
                                    "h-auto min-h-[60px] w-full rounded-md border border-input bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground",
                                    "cursor-pointer transition-colors focus:outline-none focus:ring-1 focus:ring-ring",
                                )}
                                multiple
                                value={value.subtitleLangs}
                                onChange={(e) => {
                                    const selected = Array.from(
                                        e.target.selectedOptions,
                                        (o) => o.value,
                                    );
                                    onChange({ ...value, subtitleLangs: selected });
                                }}
                            >
                                {availableSubtitleLangs.map((lang) => (
                                    <option key={lang} value={lang}>
                                        {lang}
                                    </option>
                                ))}
                            </select>
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
                            onChange({ ...value, embedThumbnail: checked === true })
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
        </div>
    );
}
