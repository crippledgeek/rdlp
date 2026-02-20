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

export function SettingsPage() {
    const { data: settings, isLoading: settingsLoading } = useQuery(settingsQueryOptions());
    const { data: providers = [] } = useQuery(providersQueryOptions());
    const [draft, setDraft] = useState<AppSettings | null>(null);

    useEffect(() => {
        if (settings) {
            setDraft(settings);
        }
    }, [settings]);

    if (settingsLoading || !draft) {
        return <div className="flex items-center justify-center py-16 text-muted-foreground">Loading settings...</div>;
    }

    const [saveError, setSaveError] = useState<string | null>(null);

    const handleSave = async () => {
        try {
            setSaveError(null);
            await updateSettings(draft);
        } catch (err: unknown) {
            const msg = err instanceof Error ? err.message : String(err);
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
                <select
                    className="flex h-9 w-full rounded-md border border-input bg-card px-3 py-1 text-sm text-foreground shadow-sm transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                    value={draft.default_remux ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_remux:
                                (e.target.value as ContainerFormat) || null,
                        })
                    }
                >
                    <option value="">None</option>
                    <option value="mp4">MP4</option>
                    <option value="mkv">MKV</option>
                    <option value="webm">WebM</option>
                </select>
            </div>

            <div className="mb-4">
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Default Audio Extraction</Label>
                <select
                    className="flex h-9 w-full rounded-md border border-input bg-card px-3 py-1 text-sm text-foreground shadow-sm transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                    value={draft.default_extract_audio ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_extract_audio:
                                (e.target.value as AudioFormat) || null,
                        })
                    }
                >
                    <option value="">None</option>
                    <option value="mp3">MP3</option>
                    <option value="aac">AAC</option>
                    <option value="opus">Opus</option>
                    <option value="flac">FLAC</option>
                </select>
            </div>

            <div className="mb-4">
                <Label className="text-[13px] font-semibold text-foreground mb-1.5 block">Default Subtitle Format</Label>
                <select
                    className="flex h-9 w-full rounded-md border border-input bg-card px-3 py-1 text-sm text-foreground shadow-sm transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                    value={draft.default_subtitle_format ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_subtitle_format:
                                (e.target.value as SubtitleFormat) || null,
                        })
                    }
                >
                    <option value="">None</option>
                    <option value="srt">SRT</option>
                    <option value="vtt">VTT</option>
                    <option value="ass">ASS</option>
                </select>
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
                <select
                    className="flex h-9 w-full rounded-md border border-input bg-card px-3 py-1 text-sm text-foreground shadow-sm transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                    value={draft.default_search_provider ?? ""}
                    onChange={(e) =>
                        setDraft({
                            ...draft,
                            default_search_provider:
                                e.target.value || null,
                        })
                    }
                >
                    <option value="">Auto</option>
                    {providers.map((p) => (
                        <option key={p.name} value={p.name}>
                            {p.display_name}
                        </option>
                    ))}
                </select>
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
