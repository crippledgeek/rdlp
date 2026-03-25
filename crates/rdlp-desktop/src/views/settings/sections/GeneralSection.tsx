// GeneralSection: default search provider and format defaults.

import { Film, Search } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Select, SelectTrigger, SelectValue, SelectItem } from "@/components/ui/select";
import { SelectPopover, SelectListBox } from "@/components/ui/select";
import { providersQueryOptions } from "@/api/search";
import type { AppSettings, ContainerFormat, AudioFormat, SubtitleFormat } from "@/types";

// Jolly/React Aria Select uses selectedKey (string) and onSelectionChange (Key -> void)
// Items use 'id' not 'value'.
// We use NONE_KEY to represent "no selection" since React Aria Key must be non-empty.

const NONE_KEY = "none";

interface Props {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

interface SimpleSelectProps {
    label: string;
    value: string | null;
    onChange: (val: string | null) => void;
    items: { id: string; label: string }[];
    className?: string;
}

function SimpleSelect({ label: _label, value, onChange, items }: SimpleSelectProps) {
    return (
        <Select
            selectedKey={value ?? NONE_KEY}
            onSelectionChange={(key) => {
                const k = String(key);
                onChange(k === NONE_KEY ? null : k);
            }}
        >
            <SelectTrigger className="w-full text-sm">
                <SelectValue />
            </SelectTrigger>
            <SelectPopover>
                <SelectListBox>
                    {items.map((item) => (
                        <SelectItem id={item.id} key={item.id}>
                            {item.label}
                        </SelectItem>
                    ))}
                </SelectListBox>
            </SelectPopover>
        </Select>
    );
}

export function GeneralSection({ draft, onChange }: Props) {
    const { data: providers = [] } = useQuery(providersQueryOptions());

    return (
        <>
            {/* Search */}
            <section id="settings-general" aria-labelledby="settings-general-heading" className="settings-panel">
                <h3 id="settings-general-heading" className="settings-panel-title">
                    <Search className="size-3.5" />
                    Search
                </h3>
                <div>
                    <Label className="settings-label">Default Search Provider</Label>
                    <SimpleSelect
                        label="Default Search Provider"
                        value={draft.default_search_provider}
                        onChange={(val) => onChange({ default_search_provider: val })}
                        items={[
                            { id: NONE_KEY, label: "Auto" },
                            ...providers.map((p) => ({ id: p.name, label: p.display_name })),
                        ]}
                    />
                </div>
            </section>

            {/* Format Defaults */}
            <section id="settings-formats" aria-labelledby="settings-formats-heading" className="settings-panel">
                <h3 id="settings-formats-heading" className="settings-panel-title">
                    <Film className="size-3.5" />
                    Format Defaults
                </h3>
                <div className="grid grid-cols-2 gap-x-4 gap-y-3">
                    <div>
                        <Label className="settings-label">Remux Format</Label>
                        <SimpleSelect
                            label="Remux Format"
                            value={draft.default_remux}
                            onChange={(val) => onChange({ default_remux: val as ContainerFormat | null })}
                            items={[
                                { id: NONE_KEY, label: "None" },
                                { id: "mp4", label: "MP4" },
                                { id: "mkv", label: "MKV" },
                                { id: "webm", label: "WebM" },
                            ]}
                        />
                    </div>
                    <div>
                        <Label className="settings-label">Audio Extraction</Label>
                        <SimpleSelect
                            label="Audio Extraction"
                            value={draft.default_extract_audio}
                            onChange={(val) => onChange({ default_extract_audio: val as AudioFormat | null })}
                            items={[
                                { id: NONE_KEY, label: "None" },
                                { id: "mp3", label: "MP3" },
                                { id: "aac", label: "AAC" },
                                { id: "opus", label: "Opus" },
                                { id: "flac", label: "FLAC" },
                            ]}
                        />
                    </div>
                    <div>
                        <Label className="settings-label">Subtitle Format</Label>
                        <SimpleSelect
                            label="Subtitle Format"
                            value={draft.default_subtitle_format}
                            onChange={(val) => onChange({ default_subtitle_format: val as SubtitleFormat | null })}
                            items={[
                                { id: NONE_KEY, label: "None" },
                                { id: "srt", label: "SRT" },
                                { id: "vtt", label: "VTT" },
                                { id: "ass", label: "ASS" },
                            ]}
                        />
                    </div>
                    <div>
                        <Label className="settings-label">Subtitle Languages</Label>
                        <Input
                            type="text"
                            placeholder="en,sv,ja"
                            value={draft.default_subtitle_langs.join(",")}
                            onChange={(e) =>
                                onChange({
                                    default_subtitle_langs: e.target.value
                                        .split(",")
                                        .map((s) => s.trim())
                                        .filter(Boolean),
                                })
                            }
                        />
                    </div>
                </div>
            </section>
        </>
    );
}
