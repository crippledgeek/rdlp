// Audio normalization settings section extracted from SettingsPage.
// Renders the normalize-audio checkbox, mode toggle, loudnorm options,
// and boost fallback controls.

import { cn } from "@/lib/utils";
import type { AppSettings } from "../types";
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

interface AudioNormalizationSectionProps {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

export function AudioNormalizationSection({ draft, onChange }: AudioNormalizationSectionProps) {
    return (
        <>
            <h3
                id="audio-norm-heading"
                className="text-sm font-bold text-foreground mb-3"
            >
                Audio Normalization
            </h3>

            <section aria-labelledby="audio-norm-heading">
                <div className="settings-toggle-row mb-3">
                    <Checkbox
                        id="normalize-audio"
                        checked={draft.normalize_audio}
                        onCheckedChange={(checked) =>
                            onChange({ normalize_audio: checked === true })
                        }
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
                                        if (val) onChange({ loudnorm: val === "loudnorm" });
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

                        {/* Peak-specific options */}
                        {!draft.loudnorm && (
                            <div>
                                <Label
                                    htmlFor="audio-gain-target"
                                    className="settings-label"
                                >
                                    Peak Target (dBFS)
                                </Label>
                                <Input
                                    id="audio-gain-target"
                                    type="number"
                                    step="0.1"
                                    min="-30"
                                    max="0"
                                    placeholder="-1.0"
                                    value={draft.audio_gain_target ?? ""}
                                    onChange={(e) =>
                                        onChange({
                                            audio_gain_target: e.target.value
                                                ? Number(e.target.value)
                                                : null,
                                        })
                                    }
                                    className="w-32 font-mono text-xs"
                                />
                            </div>
                        )}

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
                                            onChange({
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
                                                    onChange({
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
                                                    onChange({
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
                                                    onChange({
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
                                <div className="settings-toggle-row">
                                    <Checkbox
                                        id="loudnorm-dynamic"
                                        checked={draft.loudnorm_dynamic}
                                        onCheckedChange={(checked) =>
                                            onChange({ loudnorm_dynamic: checked === true })
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
                                <div className="settings-toggle-row">
                                    <Checkbox
                                        id="loudnorm-precompress"
                                        checked={draft.loudnorm_precompress}
                                        onCheckedChange={(checked) =>
                                            onChange({ loudnorm_precompress: checked === true })
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
                        <div className="settings-toggle-row">
                            <Checkbox
                                id="normalize-boost"
                                checked={draft.normalize_boost}
                                onCheckedChange={(checked) =>
                                    onChange({ normalize_boost: checked === true })
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
                                        onChange({
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
            </section>
        </>
    );
}
