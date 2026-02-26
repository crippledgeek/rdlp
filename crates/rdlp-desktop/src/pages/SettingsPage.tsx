import { cn } from "@/lib/utils";
import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { settingsQueryOptions, updateSettings, pickDirectory } from "../api/settings";
import { providersQueryOptions } from "../api/search";
import type {
    AppSettings,
    AudioFormat,
    ContainerFormat,
    SubtitleFormat,
} from "../types";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import {
    Collapsible,
    CollapsibleContent,
} from "@/components/ui/collapsible";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
    Tooltip,
    TooltipContent,
    TooltipProvider,
    TooltipTrigger,
} from "@/components/ui/tooltip";

/** Sentinel value for "no selection" since Radix Select does not support empty string values. */
const NONE_SENTINEL = "none";

export function SettingsPage() {
    const { data: settings, isLoading: settingsLoading } = useQuery(settingsQueryOptions());
    const { data: providers = [] } = useQuery(providersQueryOptions());
    const [draft, setDraft] = useState<AppSettings | null>(null);
    const [saveError, setSaveError] = useState<string | null>(null);

    useEffect(() => {
        if (settings) {
            setDraft(settings);
        }
    }, [settings]);

    if (settingsLoading || !draft) {
        return <div className="flex items-center justify-center py-16 text-muted-foreground">Loading settings...</div>;
    }

    const handleSave = async () => {
        // Range validation for loudnorm numeric fields
        if (draft.loudnorm_target_i !== null && (draft.loudnorm_target_i < -70 || draft.loudnorm_target_i > 0)) {
            setSaveError("Loudness Target must be between -70 and 0 LUFS.");
            return;
        }
        if (draft.loudnorm_target_tp !== null && (draft.loudnorm_target_tp < -9 || draft.loudnorm_target_tp > 0)) {
            setSaveError("True Peak Limit must be between -9 and 0 dBTP.");
            return;
        }
        if (draft.loudnorm_target_lra !== null && (draft.loudnorm_target_lra < 1 || draft.loudnorm_target_lra > 30)) {
            setSaveError("Loudness Range must be between 1 and 30 LU.");
            return;
        }
        if (draft.normalize_boost_db !== null && (draft.normalize_boost_db < 0 || draft.normalize_boost_db > 30)) {
            setSaveError("Boost Gain must be between 0 and 30 dB.");
            return;
        }
        try {
            setSaveError(null);
            await updateSettings(draft);
        } catch (err: unknown) {
            const msg =
                err instanceof Error
                    ? err.message
                    : typeof err === "object" && err !== null && "message" in err && typeof (err as Record<string, unknown>).message === "string"
                      ? (err as Record<string, string>).message
                      : String(err);
            setSaveError(msg);
        }
    };

    const handlePickDir = async () => {
        const dir = await pickDirectory();
        if (dir) {
            setDraft({ ...draft, output_dir: dir });
        }
    };

    return (
        <div className="max-w-xl">
            <h2 className="text-lg font-bold mb-4 text-foreground">Settings</h2>

            <div className="mb-4">
                <Label className="settings-label">Output Directory</Label>
                <div className="flex gap-1.5">
                    <Input type="text" value={draft.output_dir} readOnly className="flex-1 font-mono text-xs" />
                    <Button variant="outline" onClick={handlePickDir}>Browse</Button>
                </div>
            </div>

            <div className="mb-4">
                <Label className="settings-label">Default Remux Format</Label>
                <Select
                    value={draft.default_remux ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        setDraft({
                            ...draft,
                            default_remux:
                                val === NONE_SENTINEL ? null : (val as ContainerFormat),
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
                        setDraft({
                            ...draft,
                            default_extract_audio:
                                val === NONE_SENTINEL ? null : (val as AudioFormat),
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

            <div className="mb-4">
                <Label className="settings-label">Default Subtitle Format</Label>
                <Select
                    value={draft.default_subtitle_format ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        setDraft({
                            ...draft,
                            default_subtitle_format:
                                val === NONE_SENTINEL ? null : (val as SubtitleFormat),
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
                        setDraft({
                            ...draft,
                            default_subtitle_langs: e.target.value
                                .split(",")
                                .map((s) => s.trim())
                                .filter(Boolean),
                        })
                    }
                />
            </div>

            <div className="mb-4">
                <Label className="settings-label">Default Search Provider</Label>
                <Select
                    value={draft.default_search_provider ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        setDraft({
                            ...draft,
                            default_search_provider:
                                val === NONE_SENTINEL ? null : val,
                        })
                    }
                >
                    <SelectTrigger className={cn("w-full text-sm", draft.default_search_provider && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>Auto</SelectItem>
                        {providers.map((p) => (
                            <SelectItem key={p.name} value={p.name}>
                                {p.display_name}
                            </SelectItem>
                        ))}
                    </SelectContent>
                </Select>
            </div>

            <div
                className="settings-toggle-row mb-4"
                onClick={() => setDraft({ ...draft, embed_thumbnail: !draft.embed_thumbnail })}
            >
                <Checkbox
                    checked={draft.embed_thumbnail}
                    onCheckedChange={(checked) => setDraft({ ...draft, embed_thumbnail: checked === true })}
                />
                <Label className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Embed thumbnails
                </Label>
            </div>

            <div
                className="settings-toggle-row mb-4"
                onClick={() => setDraft({ ...draft, embed_metadata: !draft.embed_metadata })}
            >
                <Checkbox
                    checked={draft.embed_metadata}
                    onCheckedChange={(checked) => setDraft({ ...draft, embed_metadata: checked === true })}
                />
                <Label className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Embed metadata
                </Label>
            </div>

            <div
                className="settings-toggle-row mb-4"
                onClick={() => setDraft({ ...draft, verbose: !draft.verbose })}
            >
                <Checkbox
                    id="verbose"
                    checked={draft.verbose}
                    onCheckedChange={(checked) => setDraft({ ...draft, verbose: checked === true })}
                />
                <Label htmlFor="verbose" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Verbose logging
                </Label>
            </div>

            <Separator className="my-6" />

            <h3 className="text-sm font-bold text-foreground mb-3">Audio Normalization</h3>

            <div
                className="settings-toggle-row mb-3"
                onClick={() => setDraft({ ...draft, normalize_audio: !draft.normalize_audio })}
            >
                <Checkbox
                    id="normalize-audio"
                    checked={draft.normalize_audio}
                    onCheckedChange={(checked) => setDraft({ ...draft, normalize_audio: checked === true })}
                />
                <Label htmlFor="normalize-audio" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Normalize audio
                </Label>
            </div>

            <Collapsible open={draft.normalize_audio}>
                <CollapsibleContent className="pl-4 border-l-2 border-border space-y-3 overflow-hidden">
                    {/* Mode: Peak vs EBU R128 Loudnorm */}
                    <div>
                        <Label
                            id="normalize-mode-label"
                            className="settings-label"
                        >
                            Mode
                        </Label>
                        <TooltipProvider>
                            <ToggleGroup
                                type="single"
                                variant="outline"
                                spacing={0}
                                value={draft.loudnorm ? "loudnorm" : "peak"}
                                onValueChange={(val) => {
                                    if (val) setDraft({ ...draft, loudnorm: val === "loudnorm" });
                                }}
                                aria-labelledby="normalize-mode-label"
                                className="w-fit"
                            >
                                <ToggleGroupItem value="peak" size="sm" className="text-xs px-3 data-[state=on]:select-active data-[state=on]:bg-primary/15">
                                    Peak
                                </ToggleGroupItem>
                                <Tooltip>
                                    <TooltipTrigger asChild>
                                        <span>
                                            <ToggleGroupItem value="loudnorm" size="sm" className="text-xs px-3 data-[state=on]:select-active data-[state=on]:bg-primary/15">
                                                EBU R128 Loudnorm
                                            </ToggleGroupItem>
                                        </span>
                                    </TooltipTrigger>
                                    <TooltipContent side="top">
                                        Two-pass loudness normalization to a target LUFS level
                                    </TooltipContent>
                                </Tooltip>
                            </ToggleGroup>
                        </TooltipProvider>
                    </div>

                    {/* Loudnorm-specific options */}
                    {draft.loudnorm && (
                        <>
                            {/* Preset */}
                            <div>
                                <Label
                                    htmlFor="loudnorm-preset"
                                    className="settings-label"
                                >
                                    Preset
                                </Label>
                                <Select
                                    value={draft.loudnorm_preset ?? NONE_SENTINEL}
                                    onValueChange={(val) =>
                                        setDraft({
                                            ...draft,
                                            loudnorm_preset: val === NONE_SENTINEL ? null : val,
                                        })
                                    }
                                >
                                    <SelectTrigger id="loudnorm-preset" className={cn("w-full text-sm", draft.loudnorm_preset && "select-active")}>
                                        <SelectValue />
                                    </SelectTrigger>
                                    <SelectContent>
                                        <SelectItem value={NONE_SENTINEL}>Default (Streaming)</SelectItem>
                                        <SelectItem value="streaming">Streaming (-14 LUFS)</SelectItem>
                                        <SelectItem value="broadcast">Broadcast (-23 LUFS)</SelectItem>
                                        <SelectItem value="loud">Loud (-11 LUFS)</SelectItem>
                                    </SelectContent>
                                </Select>
                            </div>

                            {/* Custom targets */}
                            <div>
                                <Label className="settings-label">
                                    Custom Targets (override preset)
                                </Label>
                                <div className="grid grid-cols-3 gap-2">
                                    <div>
                                        <Label
                                            htmlFor="loudnorm-target-i"
                                            className="text-[11px] text-muted-foreground mb-1 block"
                                        >
                                            Loudness Target (LUFS)
                                        </Label>
                                        <Input
                                            id="loudnorm-target-i"
                                            type="number"
                                            step="0.1"
                                            min="-70"
                                            max="0"
                                            placeholder={
                                                draft.loudnorm_preset === "broadcast"
                                                    ? "-23.0"
                                                    : draft.loudnorm_preset === "loud"
                                                      ? "-11.0"
                                                      : "-14.0"
                                            }
                                            value={draft.loudnorm_target_i ?? ""}
                                            onChange={(e) =>
                                                setDraft({
                                                    ...draft,
                                                    loudnorm_target_i: e.target.value
                                                        ? Number(e.target.value)
                                                        : null,
                                                })
                                            }
                                            className="font-mono text-xs"
                                        />
                                    </div>
                                    <div>
                                        <Label
                                            htmlFor="loudnorm-target-tp"
                                            className="text-[11px] text-muted-foreground mb-1 block"
                                        >
                                            True Peak Limit (dBTP)
                                        </Label>
                                        <Input
                                            id="loudnorm-target-tp"
                                            type="number"
                                            step="0.1"
                                            min="-9"
                                            max="0"
                                            placeholder={
                                                draft.loudnorm_preset === "broadcast" ? "-2.0" : "-1.0"
                                            }
                                            value={draft.loudnorm_target_tp ?? ""}
                                            onChange={(e) =>
                                                setDraft({
                                                    ...draft,
                                                    loudnorm_target_tp: e.target.value
                                                        ? Number(e.target.value)
                                                        : null,
                                                })
                                            }
                                            className="font-mono text-xs"
                                        />
                                    </div>
                                    <div>
                                        <Label
                                            htmlFor="loudnorm-target-lra"
                                            className="text-[11px] text-muted-foreground mb-1 block"
                                        >
                                            Loudness Range (LU)
                                        </Label>
                                        <Input
                                            id="loudnorm-target-lra"
                                            type="number"
                                            step="0.1"
                                            min="1"
                                            max="30"
                                            placeholder={
                                                draft.loudnorm_preset === "broadcast" ? "7.0" : "11.0"
                                            }
                                            value={draft.loudnorm_target_lra ?? ""}
                                            onChange={(e) =>
                                                setDraft({
                                                    ...draft,
                                                    loudnorm_target_lra: e.target.value
                                                        ? Number(e.target.value)
                                                        : null,
                                                })
                                            }
                                            className="font-mono text-xs"
                                        />
                                    </div>
                                </div>
                            </div>

                            {/* Dynamic mode */}
                            <div
                                className="settings-toggle-row"
                                onClick={() =>
                                    setDraft({ ...draft, loudnorm_dynamic: !draft.loudnorm_dynamic })
                                }
                            >
                                <Checkbox
                                    id="loudnorm-dynamic"
                                    checked={draft.loudnorm_dynamic}
                                    onCheckedChange={(checked) =>
                                        setDraft({ ...draft, loudnorm_dynamic: checked === true })
                                    }
                                />
                                <Label
                                    htmlFor="loudnorm-dynamic"
                                    className="text-sm font-medium text-muted-foreground cursor-pointer"
                                >
                                    Dynamic mode (per-frame compression)
                                </Label>
                            </div>

                            {/* Precompress */}
                            <div
                                className="settings-toggle-row"
                                onClick={() =>
                                    setDraft({ ...draft, loudnorm_precompress: !draft.loudnorm_precompress })
                                }
                            >
                                <Checkbox
                                    id="loudnorm-precompress"
                                    checked={draft.loudnorm_precompress}
                                    onCheckedChange={(checked) =>
                                        setDraft({ ...draft, loudnorm_precompress: checked === true })
                                    }
                                />
                                <Label
                                    htmlFor="loudnorm-precompress"
                                    className="text-sm font-medium text-muted-foreground cursor-pointer"
                                >
                                    Precompress (tame extreme peaks)
                                </Label>
                            </div>
                        </>
                    )}

                    {/* Boost fallback — visible whenever normalize_audio is true */}
                    <div
                        className="settings-toggle-row"
                        onClick={() =>
                            setDraft({ ...draft, normalize_boost: !draft.normalize_boost })
                        }
                    >
                        <Checkbox
                            id="normalize-boost"
                            checked={draft.normalize_boost}
                            onCheckedChange={(checked) =>
                                setDraft({ ...draft, normalize_boost: checked === true })
                            }
                        />
                        <Label
                            htmlFor="normalize-boost"
                            className="text-sm font-medium text-muted-foreground cursor-pointer"
                        >
                            Boost fallback (quiet/compressed audio)
                        </Label>
                    </div>

                    {draft.normalize_boost && (
                        <div>
                            <Label
                                htmlFor="normalize-boost-db"
                                className="settings-label"
                            >
                                Boost Gain (dB)
                            </Label>
                            <Input
                                id="normalize-boost-db"
                                type="number"
                                step="0.5"
                                min="0"
                                max="30"
                                placeholder="12.0"
                                value={draft.normalize_boost_db ?? ""}
                                onChange={(e) =>
                                    setDraft({
                                        ...draft,
                                        normalize_boost_db: e.target.value
                                            ? Number(e.target.value)
                                            : null,
                                    })
                                }
                                className="w-32 font-mono text-xs"
                            />
                        </div>
                    )}
                </CollapsibleContent>
            </Collapsible>

            {saveError && (
                <Alert variant="destructive" className="mb-4">
                    <AlertDescription>{saveError}</AlertDescription>
                </Alert>
            )}

            <Button onClick={handleSave} className="bg-primary text-primary-foreground hover:bg-primary/90">
                Save Settings
            </Button>
        </div>
    );
}
