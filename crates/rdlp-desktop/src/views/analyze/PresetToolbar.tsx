// PresetToolbar: quick format presets above the formats table.
// Presets: Best, 1080p, 720p, Audio Only.
// Clicking a preset auto-selects the best matching format.

import { useState } from "react";
import { Toolbar } from "@/components/ui/toolbar";
import { ToggleButton } from "react-aria-components";
import { setSelectedFormat } from "@/stores/uiStore";
import { cn } from "@/lib/utils";
import type { FormatInfo } from "@/types";

type Preset = "best" | "1080p" | "720p" | "audio";

const PRESETS: { id: Preset; label: string }[] = [
    { id: "best", label: "Best" },
    { id: "1080p", label: "1080p" },
    { id: "720p", label: "720p" },
    { id: "audio", label: "Audio Only" },
];

function findFormatForPreset(formats: FormatInfo[], preset: Preset): string | null {
    switch (preset) {
        case "best": {
            const muxed = formats.filter((f) => f.has_video && f.has_audio);
            const sorted = (muxed.length > 0 ? muxed : formats.filter((f) => f.has_video))
                .sort((a, b) => (b.height ?? 0) - (a.height ?? 0));
            return sorted[0]?.format_id ?? null;
        }
        case "1080p": {
            const candidates = formats.filter((f) => f.has_video && (f.height ?? 0) <= 1080);
            return candidates.sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0]?.format_id ?? null;
        }
        case "720p": {
            const candidates = formats.filter((f) => f.has_video && (f.height ?? 0) <= 720);
            return candidates.sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0]?.format_id ?? null;
        }
        case "audio": {
            const audioOnly = formats.filter((f) => !f.has_video && f.has_audio);
            return audioOnly[0]?.format_id ?? null;
        }
        default:
            return null;
    }
}

interface PresetToolbarProps {
    formats: FormatInfo[];
}

export function PresetToolbar({ formats }: PresetToolbarProps) {
    const [activePreset, setActivePreset] = useState<Preset>("best");

    function handlePreset(preset: Preset) {
        setActivePreset(preset);
        const formatId = findFormatForPreset(formats, preset);
        if (formatId) setSelectedFormat(formatId);
    }

    return (
        <div className="flex items-center gap-2 px-3 py-1.5 border-b border-[#1a1a2e] bg-[var(--surface-raised)] shrink-0">
            <Toolbar aria-label="Format presets" className="flex items-center gap-1">
                {PRESETS.map((preset) => (
                    <ToggleButton
                        key={preset.id}
                        isSelected={activePreset === preset.id}
                        onChange={() => handlePreset(preset.id)}
                        className={cn(
                            "px-2.5 py-0.5 rounded-[4px] text-[11px] font-medium transition-colors cursor-pointer outline-none",
                            "border border-transparent",
                            activePreset === preset.id
                                ? "bg-[#1a2a4a] text-[#4a9eff] border-[#2a3a5a]"
                                : "text-[#666666] hover:text-[#aaaaaa] hover:bg-[#0e0e1e]",
                        )}
                    >
                        {preset.label}
                    </ToggleButton>
                ))}
            </Toolbar>

            <div className="flex-1" />

            <span className="text-[10px] text-[#444444]">
                {formats.length} {formats.length === 1 ? "format" : "formats"}
            </span>
        </div>
    );
}
