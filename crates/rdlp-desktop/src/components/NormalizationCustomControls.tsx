// Expanded inline controls for custom audio normalization parameters.
//
// Used by both FormatOptionsPanel (DownloadPage) and DownloadOptionsPanel (FormatDialog).

import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { DownloadOptions } from "../types";

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
        <div className="mt-2 pl-3 border-l-2 border-border space-y-3 animate-in fade-in-0 slide-in-from-top-1 duration-150">
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
                <p className="text-[10px] text-muted-foreground/60 mt-1">
                    {value.loudnorm
                        ? "Two-pass loudness normalization (EBU R128)"
                        : "Normalize peak level to 0 dBFS"}
                </p>
            </div>

            {/* Loudnorm-specific controls */}
            {value.loudnorm && (
                <div className="space-y-2.5">
                    {/* Target values */}
                    <div>
                        <Label className="text-[10px] text-muted-foreground mb-1 block">Targets</Label>
                        <div className="grid grid-cols-3 gap-1.5">
                            <div>
                                <Label className="text-[10px] text-muted-foreground/60 mb-0.5 block">
                                    I (LUFS)
                                </Label>
                                <Input
                                    type="number"
                                    step="0.1"
                                    min="-70"
                                    max="0"
                                    placeholder="-14.0"
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
                                <Label className="text-[10px] text-muted-foreground/60 mb-0.5 block">
                                    TP (dBTP)
                                </Label>
                                <Input
                                    type="number"
                                    step="0.1"
                                    min="-9"
                                    max="0"
                                    placeholder="-1.0"
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
                                <Label className="text-[10px] text-muted-foreground/60 mb-0.5 block">
                                    LRA (LU)
                                </Label>
                                <Input
                                    type="number"
                                    step="0.1"
                                    min="1"
                                    max="30"
                                    placeholder="11.0"
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
                        <p className="text-[10px] text-muted-foreground/60 mt-1">
                            Leave blank to use defaults
                        </p>
                    </div>

                    {/* Processing options */}
                    <div className="space-y-1.5">
                        <Label className="text-[10px] text-muted-foreground block">Options</Label>
                        <div className="flex items-start gap-2">
                            <Checkbox
                                id={`${idPrefix}-dynamic`}
                                checked={value.loudnormDynamic === true}
                                onCheckedChange={(checked) =>
                                    onChange({ ...value, loudnormDynamic: checked === true })
                                }
                                className="mt-0.5"
                            />
                            <div>
                                <Label htmlFor={`${idPrefix}-dynamic`} className="text-[11px] text-muted-foreground cursor-pointer block">
                                    Dynamic mode
                                </Label>
                                <p className="text-[10px] text-muted-foreground/50">
                                    Use linear normalization instead of compression
                                </p>
                            </div>
                        </div>
                        <div className="flex items-start gap-2">
                            <Checkbox
                                id={`${idPrefix}-precompress`}
                                checked={value.loudnormPrecompress === true}
                                onCheckedChange={(checked) =>
                                    onChange({ ...value, loudnormPrecompress: checked === true })
                                }
                                className="mt-0.5"
                            />
                            <div>
                                <Label htmlFor={`${idPrefix}-precompress`} className="text-[11px] text-muted-foreground cursor-pointer block">
                                    Precompress
                                </Label>
                                <p className="text-[10px] text-muted-foreground/50">
                                    Apply gentle compression before loudnorm
                                </p>
                            </div>
                        </div>
                    </div>
                </div>
            )}

            {/* Separator between loudnorm options and boost */}
            {value.loudnorm && (
                <div className="border-t border-border/50" />
            )}

            {/* Boost fallback — always visible in custom mode */}
            <div className="space-y-1.5">
                <div className="flex items-start gap-2">
                    <Checkbox
                        id={`${idPrefix}-boost`}
                        checked={value.normalizeBoost === true}
                        onCheckedChange={(checked) =>
                            onChange({ ...value, normalizeBoost: checked === true })
                        }
                        className="mt-0.5"
                    />
                    <div className="flex-1">
                        <div className="flex items-center gap-2">
                            <Label htmlFor={`${idPrefix}-boost`} className="text-[11px] text-muted-foreground cursor-pointer">
                                Boost fallback
                            </Label>
                            {value.normalizeBoost && (
                                <div className="flex items-center gap-1">
                                    <Input
                                        type="number"
                                        step="0.5"
                                        min="0"
                                        max="30"
                                        placeholder="12"
                                        value={value.normalizeBoostDb ?? ""}
                                        onChange={(e) =>
                                            onChange({
                                                ...value,
                                                normalizeBoostDb: e.target.value ? Number(e.target.value) : null,
                                            })
                                        }
                                        className="w-16 h-5 text-[11px] font-mono"
                                    />
                                    <span className="text-[10px] text-muted-foreground/50">dB</span>
                                </div>
                            )}
                        </div>
                        <p className="text-[10px] text-muted-foreground/50">
                            Apply +dB gain with limiter if normalization is too quiet
                        </p>
                    </div>
                </div>
            </div>
        </div>
    );
}
