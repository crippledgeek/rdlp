import { cn } from "@/lib/utils";
import { Checkbox } from "@/components/ui/checkbox";
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
                    <div className="text-[11px] font-semibold text-muted-foreground tracking-wide">Quality Preset</div>
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
                <div className="text-[11px] font-semibold text-muted-foreground tracking-wide">Remux</div>
                <select
                    className="h-8 rounded-md border border-input bg-card px-2.5 pr-7 text-xs font-medium text-muted-foreground cursor-pointer appearance-none transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                    value={value.remux ?? ""}
                    onChange={(e) => handleRemux(e.target.value)}
                    style={{
                        backgroundImage: "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10' viewBox='0 0 24 24' fill='none' stroke='%236b7280' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='6 9 12 15 18 9'/%3E%3C/svg%3E\")",
                        backgroundRepeat: "no-repeat",
                        backgroundPosition: "right 8px center",
                    }}
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
                <div className="text-[11px] font-semibold text-muted-foreground tracking-wide">Extract Audio</div>
                <select
                    className="h-8 rounded-md border border-input bg-card px-2.5 pr-7 text-xs font-medium text-muted-foreground cursor-pointer appearance-none transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                    value={value.extractAudio ?? ""}
                    onChange={(e) => handleAudio(e.target.value)}
                    style={{
                        backgroundImage: "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10' viewBox='0 0 24 24' fill='none' stroke='%236b7280' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='6 9 12 15 18 9'/%3E%3C/svg%3E\")",
                        backgroundRepeat: "no-repeat",
                        backgroundPosition: "right 8px center",
                    }}
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
                <label className="flex items-center gap-2 text-[11px] font-semibold text-muted-foreground tracking-wide cursor-pointer">
                    <Checkbox
                        checked={value.subtitles}
                        onCheckedChange={(checked) => handleSubtitles(checked === true)}
                    />
                    Download Subtitles
                </label>
                {value.subtitles && (
                    <div className="mt-1.5">
                        {availableSubtitleLangs.length > 0 ? (
                            <select
                                className="h-auto min-h-[60px] w-full rounded-md border border-input bg-card px-2.5 py-1.5 text-xs font-medium text-muted-foreground cursor-pointer transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
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
                            <input
                                className="w-full py-[7px] px-2.5 border border-input rounded-md bg-muted text-foreground font-mono text-xs transition-colors placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
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
                <label className="flex items-center gap-2 text-[11px] font-semibold text-muted-foreground tracking-wide cursor-pointer">
                    <Checkbox
                        checked={value.embedThumbnail}
                        onCheckedChange={(checked) =>
                            onChange({ ...value, embedThumbnail: checked === true })
                        }
                    />
                    Embed Thumbnail
                </label>
            </div>
        </div>
    );
}
