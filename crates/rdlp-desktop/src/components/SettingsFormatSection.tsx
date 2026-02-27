import { memo } from "react";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { NONE_SENTINEL } from "./utils/formatConstants";
import type {
    AppSettings,
    AudioFormat,
    ContainerFormat,
    SubtitleFormat,
} from "../types";

interface SettingsFormatSectionProps {
    draft: AppSettings;
    onChange: (next: AppSettings) => void;
}

export const SettingsFormatSection = memo(function SettingsFormatSection({
    draft,
    onChange,
}: SettingsFormatSectionProps) {
    return (
        <>
            {/* ── Format defaults ────────────────────────────────── */}
            <div className="mb-4">
                <Label className="settings-label">Default Remux Format</Label>
                <Select
                    value={draft.default_remux ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        onChange({
                            ...draft,
                            default_remux: val === NONE_SENTINEL ? null : (val as ContainerFormat),
                        })
                    }
                >
                    <SelectTrigger className={cn("w-full text-sm", draft.default_remux && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                        <SelectItem value="mp4">MP4</SelectItem>
                        <SelectItem value="mkv">MKV</SelectItem>
                        <SelectItem value="webm">WebM</SelectItem>
                    </SelectContent>
                </Select>
            </div>

            <div className="mb-4">
                <Label className="settings-label">Default Audio Extraction</Label>
                <Select
                    value={draft.default_extract_audio ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        onChange({
                            ...draft,
                            default_extract_audio: val === NONE_SENTINEL ? null : (val as AudioFormat),
                        })
                    }
                >
                    <SelectTrigger className={cn("w-full text-sm", draft.default_extract_audio && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                        <SelectItem value="mp3">MP3</SelectItem>
                        <SelectItem value="aac">AAC</SelectItem>
                        <SelectItem value="opus">Opus</SelectItem>
                        <SelectItem value="flac">FLAC</SelectItem>
                    </SelectContent>
                </Select>
            </div>

            {/* ── Thumbnails ──────────────────────────────────────── */}
            <div className="settings-toggle-row mb-4">
                <Checkbox
                    id="embed-thumbnail"
                    checked={draft.embed_thumbnail}
                    onCheckedChange={(checked) =>
                        onChange({ ...draft, embed_thumbnail: checked === true })
                    }
                />
                <Label htmlFor="embed-thumbnail" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Embed thumbnails
                </Label>
            </div>

            <div className="settings-toggle-row mb-4">
                <Checkbox
                    id="write-thumbnail"
                    checked={draft.write_thumbnail}
                    onCheckedChange={(checked) =>
                        onChange({ ...draft, write_thumbnail: checked === true })
                    }
                />
                <Label htmlFor="write-thumbnail" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Save thumbnail to disk
                </Label>
            </div>

            {/* ── Subtitles ──────────────────────────────────────── */}
            <div className="mb-4">
                <Label className="settings-label">Default Subtitle Format</Label>
                <Select
                    value={draft.default_subtitle_format ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        onChange({
                            ...draft,
                            default_subtitle_format: val === NONE_SENTINEL ? null : (val as SubtitleFormat),
                        })
                    }
                >
                    <SelectTrigger className={cn("w-full text-sm", draft.default_subtitle_format && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                        <SelectItem value="srt">SRT</SelectItem>
                        <SelectItem value="vtt">VTT</SelectItem>
                        <SelectItem value="ass">ASS</SelectItem>
                    </SelectContent>
                </Select>
            </div>

            <div className="mb-4">
                <Label className="settings-label">Default Subtitle Languages</Label>
                <Input
                    type="text"
                    placeholder="en,sv,ja"
                    value={draft.default_subtitle_langs.join(",")}
                    onChange={(e) =>
                        onChange({
                            ...draft,
                            default_subtitle_langs: e.target.value
                                .split(",")
                                .map((s) => s.trim())
                                .filter(Boolean),
                        })
                    }
                />
            </div>

            <div className="settings-toggle-row mb-4">
                <Checkbox
                    id="embed-subtitles"
                    checked={draft.embed_subtitles}
                    onCheckedChange={(checked) =>
                        onChange({ ...draft, embed_subtitles: checked === true })
                    }
                />
                <Label htmlFor="embed-subtitles" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Embed subtitles into container
                </Label>
            </div>

            {/* ── Misc toggles ───────────────────────────────────── */}
            <div className="settings-toggle-row mb-4">
                <Checkbox
                    id="embed-metadata"
                    checked={draft.embed_metadata}
                    onCheckedChange={(checked) =>
                        onChange({ ...draft, embed_metadata: checked === true })
                    }
                />
                <Label htmlFor="embed-metadata" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Embed metadata
                </Label>
            </div>

            <div className="settings-toggle-row mb-4">
                <Checkbox
                    id="verbose"
                    checked={draft.verbose}
                    onCheckedChange={(checked) =>
                        onChange({ ...draft, verbose: checked === true })
                    }
                />
                <Label htmlFor="verbose" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Verbose logging
                </Label>
            </div>
        </>
    );
});
