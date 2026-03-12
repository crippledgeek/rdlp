import { cn } from "@/lib/utils";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import type { SubtitleFormat } from "../../types";
import type { SettingsSectionProps } from "./types";

const NONE_SENTINEL = "none";

/** Thumbnail, subtitle, and metadata settings. */
export function MediaSection({ draft, onChange }: SettingsSectionProps) {
    return (
        <>
            {/* Thumbnail */}
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

            {/* Subtitles */}
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

            {/* Misc toggles */}
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
}
