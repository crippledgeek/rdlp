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
    Tooltip,
    TooltipContent,
    TooltipProvider,
    TooltipTrigger,
} from "@/components/ui/tooltip";
import { AudioNormalizationSection } from "@/components/AudioNormalizationSection";

/** Sentinel value for "no selection" since Radix Select does not support empty string values. */
const NONE_SENTINEL = "none";

/** Validate settings before save. Returns error message or null if valid. */
export function validateSettings(draft: AppSettings): string | null {
    if (draft.loudnorm_target_i !== null && (draft.loudnorm_target_i < -70 || draft.loudnorm_target_i > 0)) {
        return "Loudness Target must be between -70 and 0 LUFS.";
    }
    if (draft.loudnorm_target_tp !== null && (draft.loudnorm_target_tp < -9 || draft.loudnorm_target_tp > 0)) {
        return "True Peak Limit must be between -9 and 0 dBTP.";
    }
    if (draft.loudnorm_target_lra !== null && (draft.loudnorm_target_lra < 1 || draft.loudnorm_target_lra > 30)) {
        return "Loudness Range must be between 1 and 30 LU.";
    }
    if (draft.normalize_boost_db !== null && (draft.normalize_boost_db < 0 || draft.normalize_boost_db > 30)) {
        return "Boost Gain must be between 0 and 30 dB.";
    }
    return null;
}

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
        const validationError = validateSettings(draft);
        if (validationError) {
            setSaveError(validationError);
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

            {/* ── Output ─────────────────────────────────────────── */}
            <div className="mb-4">
                <Label className="settings-label">Output Directory</Label>
                <div className="flex gap-1.5">
                    <Input type="text" value={draft.output_dir} readOnly className="flex-1 font-mono text-xs" />
                    <Button variant="outline" onClick={handlePickDir}>Browse</Button>
                </div>
            </div>

            <div className="mb-4">
                <div className="flex items-center gap-1.5 mb-1">
                    <Label htmlFor="output-template" className="settings-label">Output Filename Template</Label>
                    <TooltipProvider>
                        <Tooltip>
                            <TooltipTrigger asChild>
                                <span className="text-xs text-muted-foreground cursor-help underline decoration-dotted">
                                    ?
                                </span>
                            </TooltipTrigger>
                            <TooltipContent side="right" className="max-w-xs text-xs">
                                <p className="font-semibold mb-1">Common variables:</p>
                                <ul className="space-y-0.5">
                                    <li><code>%(title)s</code> — Video title</li>
                                    <li><code>%(ext)s</code> — File extension</li>
                                    <li><code>%(uploader)s</code> — Uploader name</li>
                                    <li><code>%(upload_date)s</code> — Upload date (YYYYMMDD)</li>
                                    <li><code>%(id)s</code> — Video ID</li>
                                    <li><code>%(playlist_index)s</code> — Playlist position</li>
                                </ul>
                                <p className="mt-1 text-muted-foreground">e.g. <code>%(uploader)s/%(title)s.%(ext)s</code></p>
                            </TooltipContent>
                        </Tooltip>
                    </TooltipProvider>
                </div>
                <Input
                    id="output-template"
                    type="text"
                    placeholder="%(title)s.%(ext)s"
                    value={draft.output_template ?? ""}
                    onChange={(e) =>
                        setDraft({ ...draft, output_template: e.target.value || null })
                    }
                    className="font-mono text-xs"
                />
            </div>

            <Separator className="my-6" />

            {/* ── Format defaults ────────────────────────────────── */}
            <div className="mb-4">
                <Label className="settings-label">Default Remux Format</Label>
                <Select
                    value={draft.default_remux ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        setDraft({
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
                        setDraft({
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

            {/* ── Thumbnail ──────────────────────────────────────── */}
            <div className="settings-toggle-row mb-4">
                <Checkbox
                    id="embed-thumbnail"
                    checked={draft.embed_thumbnail}
                    onCheckedChange={(checked) =>
                        setDraft({ ...draft, embed_thumbnail: checked === true })
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
                        setDraft({ ...draft, write_thumbnail: checked === true })
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
                        setDraft({
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

            <div className="settings-toggle-row mb-4">
                <Checkbox
                    id="embed-subtitles"
                    checked={draft.embed_subtitles}
                    onCheckedChange={(checked) =>
                        setDraft({ ...draft, embed_subtitles: checked === true })
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
                        setDraft({ ...draft, embed_metadata: checked === true })
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
                        setDraft({ ...draft, verbose: checked === true })
                    }
                />
                <Label htmlFor="verbose" className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Verbose logging
                </Label>
            </div>

            {/* ── Search ─────────────────────────────────────────── */}
            <div className="mb-4">
                <Label className="settings-label">Default Search Provider</Label>
                <Select
                    value={draft.default_search_provider ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        setDraft({
                            ...draft,
                            default_search_provider: val === NONE_SENTINEL ? null : val,
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

            <Separator className="my-6" />

            {/* ── Cookies ────────────────────────────────────────── */}
            <h3
                id="cookies-heading"
                className="text-sm font-bold text-foreground mb-3"
            >
                Cookies
            </h3>

            <section aria-labelledby="cookies-heading">
                <div className="mb-4">
                    <Label className="settings-label">Browser</Label>
                    <Select
                        value={draft.cookies_from_browser ?? NONE_SENTINEL}
                        onValueChange={(val) =>
                            setDraft({
                                ...draft,
                                cookies_from_browser: val === NONE_SENTINEL ? null : val,
                            })
                        }
                    >
                        <SelectTrigger className={cn("w-full text-sm", draft.cookies_from_browser && "select-active")}>
                            <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                            <SelectItem value="chrome">Chrome</SelectItem>
                            <SelectItem value="firefox">Firefox</SelectItem>
                        </SelectContent>
                    </Select>
                </div>

                <div className="mb-4">
                    <Label htmlFor="cookies-file" className="settings-label">
                        Cookie File (Netscape format)
                    </Label>
                    <Input
                        id="cookies-file"
                        type="text"
                        placeholder="/path/to/cookies.txt"
                        value={draft.cookies_file ?? ""}
                        onChange={(e) =>
                            setDraft({ ...draft, cookies_file: e.target.value || null })
                        }
                        className="font-mono text-xs"
                    />
                </div>
            </section>

            <Separator className="my-6" />

            {/* ── Network ────────────────────────────────────────── */}
            <h3
                id="network-heading"
                className="text-sm font-bold text-foreground mb-3"
            >
                Network
            </h3>

            <section aria-labelledby="network-heading">
                <div className="mb-4">
                    <Label htmlFor="proxy" className="settings-label">Proxy</Label>
                    <Input
                        id="proxy"
                        type="text"
                        placeholder="http://proxy:8080"
                        value={draft.proxy ?? ""}
                        onChange={(e) =>
                            setDraft({ ...draft, proxy: e.target.value || null })
                        }
                        className="font-mono text-xs"
                    />
                </div>

                <div className="mb-4">
                    <Label htmlFor="rate-limit" className="settings-label">Rate Limit</Label>
                    <Input
                        id="rate-limit"
                        type="text"
                        placeholder="500K, 2M"
                        value={draft.rate_limit ?? ""}
                        onChange={(e) =>
                            setDraft({ ...draft, rate_limit: e.target.value || null })
                        }
                        className="font-mono text-xs"
                    />
                </div>
            </section>

            <Separator className="my-6" />

            {/* ── Audio Normalization ────────────────────────────── */}
            <AudioNormalizationSection
                draft={draft}
                onChange={(update) => setDraft({ ...draft, ...update })}
            />

            {saveError && (
                <Alert variant="destructive" className="my-4">
                    <AlertDescription>{saveError}</AlertDescription>
                </Alert>
            )}

            <Button onClick={handleSave} className="bg-primary text-primary-foreground hover:bg-primary/90">
                Save Settings
            </Button>
        </div>
    );
}
