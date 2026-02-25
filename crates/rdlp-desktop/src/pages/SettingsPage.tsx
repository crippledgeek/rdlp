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
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Output Directory</Label>
                <div className="flex gap-1.5">
                    <Input type="text" value={draft.output_dir} readOnly className="flex-1 font-mono text-xs" />
                    <Button variant="outline" onClick={handlePickDir}>Browse</Button>
                </div>
            </div>

            <div className="mb-4">
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Default Remux Format</Label>
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
                    <SelectTrigger className="w-full text-sm">
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
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Default Audio Extraction</Label>
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
                    <SelectTrigger className="w-full text-sm">
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
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Default Subtitle Format</Label>
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
                    <SelectTrigger className="w-full text-sm">
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
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Default Subtitle Languages</Label>
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
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Default Search Provider</Label>
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
                    <SelectTrigger className="w-full text-sm">
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
                className="flex items-center gap-2.5 p-2.5 px-3 bg-card border border-border rounded-md cursor-pointer hover:border-white/[0.08] hover:bg-muted transition-colors mb-4"
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
                className="flex items-center gap-2.5 p-2.5 px-3 bg-card border border-border rounded-md cursor-pointer hover:border-white/[0.08] hover:bg-muted transition-colors mb-4"
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
                className="flex items-center gap-2.5 p-2.5 px-3 bg-card border border-border rounded-md cursor-pointer hover:border-white/[0.08] hover:bg-muted transition-colors mb-4"
                onClick={() => setDraft({ ...draft, verbose: !draft.verbose })}
            >
                <Checkbox
                    checked={draft.verbose}
                    onCheckedChange={(checked) => setDraft({ ...draft, verbose: checked === true })}
                />
                <Label className="text-sm font-medium text-muted-foreground cursor-pointer">
                    Verbose logging
                </Label>
            </div>

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
