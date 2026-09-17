// NormalizationSection: audio normalization settings.

import { Volume2 } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import {
    Select,
    SelectItem,
    SelectListBox,
    SelectPopover,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { ToggleButton } from "react-aria-components";
import { cn } from "@/lib/utils";
import type { AppSettings, EffectiveNormalize, LoudnormPreset } from "@/types";

const NONE_KEY = "none";

/** Every preset the engine knows, in the order the Select lists them. */
const PRESETS: readonly { id: LoudnormPreset; label: string }[] = [
    { id: "streaming", label: "Streaming" },
    { id: "broadcast", label: "Broadcast" },
    { id: "loud", label: "Loud" },
];

/** Title-case a wire preset for the inherit entry's label. */
function presetLabel(preset: LoudnormPreset): string {
    return PRESETS.find((p) => p.id === preset)?.label ?? preset;
}

interface Props {
    draft: AppSettings;
    /**
     * What an empty (inherit) field resolves to, for the DRAFT'S preset —
     * fetched over IPC (`effective_normalize`). Every numeric placeholder
     * here derives from it; the section holds no copy of a default (#611).
     */
    effective: EffectiveNormalize;
    onChange: (update: Partial<AppSettings>) => void;
}

export function NormalizationSection({ draft, effective, onChange }: Props) {
    return (
        <section id="settings-normalization" aria-labelledby="settings-normalization-heading" className="settings-panel">
            <h3 id="settings-normalization-heading" className="settings-panel-title">
                <Volume2 className="size-3.5" />
                Audio Normalization
            </h3>

            <div className="space-y-3">
                {/* Enable toggle */}
                <div className="settings-toggle-row">
                    <Checkbox
                        id="normalize-audio"
                        isSelected={draft.normalize_audio}
                        onChange={(checked) => onChange({ normalize_audio: checked })}
                    >
                        <Label htmlFor="normalize-audio" className="text-sm font-medium text-muted-foreground cursor-pointer">
                            Normalize audio
                        </Label>
                    </Checkbox>
                </div>

                {draft.normalize_audio && (
                    <div className="pl-4 border-l-2 border-border space-y-3">
                        {/* Mode toggle */}
                        <div>
                            <p className="settings-label mb-1">Mode</p>
                            <div className="flex gap-1">
                                <ToggleButton
                                    isSelected={!draft.loudnorm}
                                    onChange={() => onChange({ loudnorm: false })}
                                    className={cn(
                                        "px-3 py-1 text-xs rounded-[4px] border transition-colors",
                                        !draft.loudnorm
                                            ? "bg-[#1a2a4a] border-[#4a9eff] text-[#4a9eff]"
                                            : "bg-transparent border-[#2a2a3e] text-[var(--text-muted)] hover:text-[#aaaaaa]",
                                    )}
                                >
                                    Peak
                                </ToggleButton>
                                <ToggleButton
                                    isSelected={!!draft.loudnorm}
                                    onChange={() => onChange({ loudnorm: true })}
                                    className={cn(
                                        "px-3 py-1 text-xs rounded-[4px] border transition-colors",
                                        draft.loudnorm
                                            ? "bg-[#1a2a4a] border-[#4a9eff] text-[#4a9eff]"
                                            : "bg-transparent border-[#2a2a3e] text-[var(--text-muted)] hover:text-[#aaaaaa]",
                                    )}
                                >
                                    EBU R128 Loudnorm
                                </ToggleButton>
                            </div>
                        </div>

                        {/* Peak options */}
                        {!draft.loudnorm && (
                            <div>
                                <Label htmlFor="audio-gain-target" className="settings-label">
                                    Peak Target (dBFS)
                                </Label>
                                <Input
                                    id="audio-gain-target"
                                    type="number"
                                    step="0.1"
                                    min="-30"
                                    max="0"
                                    placeholder={String(effective.peak_target_db)}
                                    value={draft.audio_gain_target ?? ""}
                                    onChange={(e) =>
                                        onChange({ audio_gain_target: e.target.value ? Number(e.target.value) : null })
                                    }
                                    className="w-32 font-mono text-xs"
                                />
                            </div>
                        )}

                        {/* Loudnorm options */}
                        {draft.loudnorm && (
                            <>
                                <div>
                                    <Label className="settings-label">Preset</Label>
                                    <Select
                                        selectedKey={draft.loudnorm_preset ?? NONE_KEY}
                                        onSelectionChange={(key) =>
                                            onChange({
                                                loudnorm_preset: key === NONE_KEY ? null : (String(key) as LoudnormPreset),
                                            })
                                        }
                                    >
                                        <SelectTrigger className="w-full text-sm">
                                            <SelectValue />
                                        </SelectTrigger>
                                        <SelectPopover>
                                            <SelectListBox>
                                                {/* The inherit entry names the preset the engine resolved to, not a typed default. */}
                                                <SelectItem id={NONE_KEY}>{`Default (${presetLabel(effective.preset)})`}</SelectItem>
                                                {PRESETS.map(({ id, label }) => (
                                                    <SelectItem key={id} id={id}>{label}</SelectItem>
                                                ))}
                                            </SelectListBox>
                                        </SelectPopover>
                                    </Select>
                                </div>
                                <div className="grid grid-cols-3 gap-2">
                                    {[
                                        { id: "loudnorm-target-i", label: "Loudness (LUFS)", field: "loudnorm_target_i" as const, placeholder: String(effective.target_i) },
                                        { id: "loudnorm-target-tp", label: "True Peak (dBTP)", field: "loudnorm_target_tp" as const, placeholder: String(effective.target_tp) },
                                        { id: "loudnorm-target-lra", label: "Range (LU)", field: "loudnorm_target_lra" as const, placeholder: String(effective.target_lra) },
                                    ].map(({ id, label, field, placeholder }) => (
                                        <div key={id}>
                                            <Label htmlFor={id} className="text-[11px] text-muted-foreground mb-1 block">{label}</Label>
                                            <Input
                                                id={id}
                                                type="number"
                                                step="0.1"
                                                placeholder={placeholder}
                                                value={draft[field] ?? ""}
                                                onChange={(e) => onChange({ [field]: e.target.value ? Number(e.target.value) : null })}
                                                className="font-mono text-xs"
                                            />
                                        </div>
                                    ))}
                                </div>
                                <div className="settings-toggle-row">
                                    <Checkbox
                                        id="loudnorm-dynamic"
                                        isSelected={draft.loudnorm_dynamic}
                                        onChange={(checked) => onChange({ loudnorm_dynamic: checked })}
                                    >
                                        <Label htmlFor="loudnorm-dynamic" className="text-sm font-medium text-muted-foreground cursor-pointer">
                                            Dynamic mode (per-frame compression)
                                        </Label>
                                    </Checkbox>
                                </div>
                                <div className="settings-toggle-row">
                                    <Checkbox
                                        id="loudnorm-precompress"
                                        isSelected={draft.loudnorm_precompress}
                                        onChange={(checked) => onChange({ loudnorm_precompress: checked })}
                                    >
                                        <Label htmlFor="loudnorm-precompress" className="text-sm font-medium text-muted-foreground cursor-pointer">
                                            Precompress (tame extreme peaks)
                                        </Label>
                                    </Checkbox>
                                </div>
                            </>
                        )}

                        {/* Boost fallback */}
                        <div className="settings-toggle-row">
                            <Checkbox
                                id="normalize-boost"
                                isSelected={draft.normalize_boost}
                                onChange={(checked) => onChange({ normalize_boost: checked })}
                            >
                                <Label htmlFor="normalize-boost" className="text-sm font-medium text-muted-foreground cursor-pointer">
                                    Boost fallback (quiet/compressed audio)
                                </Label>
                            </Checkbox>
                        </div>
                        {draft.normalize_boost && (
                            <div>
                                <Label htmlFor="normalize-boost-db" className="settings-label">Boost Gain (dB)</Label>
                                <Input
                                    id="normalize-boost-db"
                                    type="number"
                                    step="0.5"
                                    min="0"
                                    max="30"
                                    placeholder={String(effective.boost_gain_db)}
                                    value={draft.normalize_boost_db ?? ""}
                                    onChange={(e) =>
                                        onChange({ normalize_boost_db: e.target.value ? Number(e.target.value) : null })
                                    }
                                    className="w-32 font-mono text-xs"
                                />
                            </div>
                        )}
                    </div>
                )}
            </div>
        </section>
    );
}
