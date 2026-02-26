// Collapsible download options section (Zone 3) for the format dialog.
//
// Contains save directory, remux, audio extraction, subtitles, and thumbnail controls.

import { cn } from "@/lib/utils";
import { ChevronUp, ChevronDown, FolderOpen, Settings2 } from "lucide-react";
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
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { optionsSummary } from "./utils/tableHelpers";
import { getNormSelectValue, handleNormSelectChange } from "./utils/normalization";
import { NormalizationCustomControls } from "./NormalizationCustomControls";
import type {
    AppSettings,
    AudioFormat,
    ContainerFormat,
    DownloadOptions,
} from "../types";

// -- Constants --------------------------------------------------------

const REMUX_OPTIONS: Array<{ value: ContainerFormat; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
];

const AUDIO_OPTIONS: Array<{ value: AudioFormat; label: string }> = [
    { value: "mp3", label: "MP3" },
    { value: "aac", label: "AAC" },
    { value: "opus", label: "Opus" },
    { value: "flac", label: "FLAC" },
];

const NONE_SENTINEL = "none";

/** Applied to SelectTrigger when a non-default value is chosen. */
const ACTIVE_SELECT = "border-primary/40 text-primary";

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
                        <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
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
                        <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                            Remux
                        </Label>
                        <Select
                            value={options.remux ?? NONE_SENTINEL}
                            onValueChange={(val) => setOptions((prev) => ({
                                ...prev,
                                remux: val === NONE_SENTINEL ? null : (val as ContainerFormat),
                            }))}
                        >
                            <SelectTrigger className={cn("h-7 text-xs", options.remux && ACTIVE_SELECT)}>
                                <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                                {REMUX_OPTIONS.map((o) => (
                                    <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                ))}
                            </SelectContent>
                        </Select>

                        {/* Extract Audio */}
                        <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
                            Audio
                        </Label>
                        <Select
                            value={options.extractAudio ?? NONE_SENTINEL}
                            onValueChange={(val) => setOptions((prev) => ({
                                ...prev,
                                extractAudio: val === NONE_SENTINEL ? null : (val as AudioFormat),
                            }))}
                        >
                            <SelectTrigger className={cn("h-7 text-xs", options.extractAudio && ACTIVE_SELECT)}>
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
                        <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
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
                        <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide">
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
                        <Label className="text-[11px] font-semibold text-muted-foreground tracking-wide self-start pt-1.5">
                            Normalize
                        </Label>
                        <div className="flex flex-col gap-0">
                            <Select
                                value={getNormSelectValue(options)}
                                onValueChange={(val) =>
                                    setOptions((prev) => handleNormSelectChange(prev, val))
                                }
                            >
                                <SelectTrigger className={cn("h-7 text-xs", getNormSelectValue(options) !== "default" && ACTIVE_SELECT)}>
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
