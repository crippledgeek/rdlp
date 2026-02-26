// Expanded inline controls for custom audio normalization parameters.
//
// Used by both FormatOptionsPanel (DownloadPage) and DownloadOptionsPanel (FormatDialog).

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
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { DownloadOptions } from "../types";

const NONE_SENTINEL = "none";

interface NormalizationCustomControlsProps {
    value: DownloadOptions;
    onChange: (next: DownloadOptions) => void;
    /** Prefix for DOM ids to avoid collisions when multiple instances exist. */
    idPrefix?: string;
}

/** Expanded inline controls for custom audio normalization. */
export function NormalizationCustomControls({
    value,
    onChange,
    idPrefix = "norm",
}: NormalizationCustomControlsProps) {
    return (
        <div className="mt-1.5 pl-3 border-l-2 border-border space-y-2 animate-in fade-in-0 slide-in-from-top-1 duration-150">
            {/* Mode: Peak vs Loudnorm */}
            <div>
                <Label className="text-[10px] text-muted-foreground mb-1 block">Mode</Label>
                <ToggleGroup
                    type="single"
                    variant="outline"
                    spacing={0}
                    value={value.loudnorm ? "loudnorm" : "peak"}
                    onValueChange={(val) => {
                        if (val) onChange({ ...value, loudnorm: val === "loudnorm" });
                    }}
                    className="w-fit"
                >
                    <ToggleGroupItem value="peak" size="sm" className="text-[11px] px-2.5 h-6">
                        Peak
                    </ToggleGroupItem>
                    <ToggleGroupItem value="loudnorm" size="sm" className="text-[11px] px-2.5 h-6">
                        EBU R128
                    </ToggleGroupItem>
                </ToggleGroup>
            </div>

            {/* Loudnorm-specific controls */}
            {value.loudnorm && (
                <>
                    {/* Preset */}
                    <div>
                        <Label className="text-[10px] text-muted-foreground mb-1 block">Preset</Label>
                        <Select
                            value={value.loudnormPreset ?? NONE_SENTINEL}
                            onValueChange={(val) =>
                                onChange({ ...value, loudnormPreset: val === NONE_SENTINEL ? null : val })
                            }
                        >
                            <SelectTrigger className="h-6 text-[11px]">
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
                    <div className="grid grid-cols-3 gap-1.5">
                        <div>
                            <Label className="text-[10px] text-muted-foreground mb-0.5 block">LUFS</Label>
                            <Input
                                type="number"
                                step="0.1"
                                min="-70"
                                max="0"
                                placeholder={
                                    value.loudnormPreset === "broadcast" ? "-23.0"
                                    : value.loudnormPreset === "loud" ? "-11.0"
                                    : "-14.0"
                                }
                                value={value.loudnormTargetI ?? ""}
                                onChange={(e) =>
                                    onChange({
                                        ...value,
                                        loudnormTargetI: e.target.value ? Number(e.target.value) : null,
                                    })
                                }
                                className="h-6 text-[11px] font-mono"
                            />
                        </div>
                        <div>
                            <Label className="text-[10px] text-muted-foreground mb-0.5 block">TP (dB)</Label>
                            <Input
                                type="number"
                                step="0.1"
                                min="-9"
                                max="0"
                                placeholder={value.loudnormPreset === "broadcast" ? "-2.0" : "-1.0"}
                                value={value.loudnormTargetTp ?? ""}
                                onChange={(e) =>
                                    onChange({
                                        ...value,
                                        loudnormTargetTp: e.target.value ? Number(e.target.value) : null,
                                    })
                                }
                                className="h-6 text-[11px] font-mono"
                            />
                        </div>
                        <div>
                            <Label className="text-[10px] text-muted-foreground mb-0.5 block">LRA (LU)</Label>
                            <Input
                                type="number"
                                step="0.1"
                                min="1"
                                max="30"
                                placeholder={value.loudnormPreset === "broadcast" ? "7.0" : "11.0"}
                                value={value.loudnormTargetLra ?? ""}
                                onChange={(e) =>
                                    onChange({
                                        ...value,
                                        loudnormTargetLra: e.target.value ? Number(e.target.value) : null,
                                    })
                                }
                                className="h-6 text-[11px] font-mono"
                            />
                        </div>
                    </div>

                    {/* Dynamic mode */}
                    <div className="flex items-center gap-2">
                        <Checkbox
                            id={`${idPrefix}-dynamic`}
                            checked={value.loudnormDynamic === true}
                            onCheckedChange={(checked) =>
                                onChange({ ...value, loudnormDynamic: checked === true })
                            }
                        />
                        <Label htmlFor={`${idPrefix}-dynamic`} className="text-[11px] text-muted-foreground cursor-pointer">
                            Dynamic mode
                        </Label>
                    </div>

                    {/* Precompress */}
                    <div className="flex items-center gap-2">
                        <Checkbox
                            id={`${idPrefix}-precompress`}
                            checked={value.loudnormPrecompress === true}
                            onCheckedChange={(checked) =>
                                onChange({ ...value, loudnormPrecompress: checked === true })
                            }
                        />
                        <Label htmlFor={`${idPrefix}-precompress`} className="text-[11px] text-muted-foreground cursor-pointer">
                            Precompress
                        </Label>
                    </div>
                </>
            )}

            {/* Boost fallback — always visible in custom mode */}
            <div className="flex items-center gap-2">
                <Checkbox
                    id={`${idPrefix}-boost`}
                    checked={value.normalizeBoost === true}
                    onCheckedChange={(checked) =>
                        onChange({ ...value, normalizeBoost: checked === true })
                    }
                />
                <Label htmlFor={`${idPrefix}-boost`} className="text-[11px] text-muted-foreground cursor-pointer">
                    Boost fallback
                </Label>
            </div>

            {value.normalizeBoost && (
                <div className="flex items-center gap-2">
                    <Label className="text-[10px] text-muted-foreground">Gain (dB)</Label>
                    <Input
                        type="number"
                        step="0.5"
                        min="0"
                        max="30"
                        placeholder="12.0"
                        value={value.normalizeBoostDb ?? ""}
                        onChange={(e) =>
                            onChange({
                                ...value,
                                normalizeBoostDb: e.target.value ? Number(e.target.value) : null,
                            })
                        }
                        className="w-20 h-6 text-[11px] font-mono"
                    />
                </div>
            )}
        </div>
    );
}
