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
        <div className="format-options-panel">
            {!hidePresets && (
                <div className="options-popover-section">
                    <div className="options-popover-label">Quality Preset</div>
                    <div className="preset-radio-group">
                        {PRESETS.map((p) => (
                            <label key={p.id}>
                                <input
                                    type="radio"
                                    name="preset"
                                    checked={currentPreset === p.id}
                                    onChange={() => handlePreset(p)}
                                />
                                {p.label}
                            </label>
                        ))}
                    </div>
                </div>
            )}

            <div className="options-popover-section">
                <div className="options-popover-label">Remux</div>
                <select
                    className="filter-select"
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

            <div className="options-popover-section">
                <div className="options-popover-label">Extract Audio</div>
                <select
                    className="filter-select"
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

            <div className="options-popover-section">
                <label className="options-popover-label" style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                    <input
                        type="checkbox"
                        checked={value.subtitles}
                        onChange={(e) => handleSubtitles(e.target.checked)}
                    />
                    Download Subtitles
                </label>
                {value.subtitles && (
                    <div style={{ marginTop: "6px" }}>
                        {availableSubtitleLangs.length > 0 ? (
                            <select
                                className="filter-select"
                                multiple
                                value={value.subtitleLangs}
                                onChange={(e) => {
                                    const selected = Array.from(
                                        e.target.selectedOptions,
                                        (o) => o.value,
                                    );
                                    onChange({ ...value, subtitleLangs: selected });
                                }}
                                style={{ minHeight: "60px", width: "100%" }}
                            >
                                {availableSubtitleLangs.map((lang) => (
                                    <option key={lang} value={lang}>
                                        {lang}
                                    </option>
                                ))}
                            </select>
                        ) : (
                            <input
                                className="format-expr-input"
                                type="text"
                                placeholder="en,sv,ja"
                                value={value.subtitleLangs.join(",")}
                                onChange={(e) => handleSubLangs(e.target.value)}
                                style={{ fontSize: "12px", padding: "7px 10px" }}
                            />
                        )}
                    </div>
                )}
            </div>

            <div className="options-popover-section">
                <label className="options-popover-label" style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                    <input
                        type="checkbox"
                        checked={value.embedThumbnail}
                        onChange={(e) =>
                            onChange({ ...value, embedThumbnail: e.target.checked })
                        }
                    />
                    Embed Thumbnail
                </label>
            </div>
        </div>
    );
}
