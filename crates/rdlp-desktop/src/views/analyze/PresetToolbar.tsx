// PresetToolbar: quick format presets above the formats table.
// Presets: Best, 1080p, 720p, Audio Only.
// Clicking a preset auto-selects the best matching format.

import { useMemo, useState } from "react";
import { Toolbar } from "@/components/ui/toolbar";
import { Checkbox, ToggleButton } from "react-aria-components";
import { useStore } from "@tanstack/react-store";
import {
    setSelectedFormat,
    setSelectedSelector,
    setShowExpertFormats,
    uiStore,
} from "@/stores/uiStore";
import { cn } from "@/lib/utils";
import type { FormatInfo } from "@/types";

type Preset = "best" | "1080p" | "720p" | "audio";

const PRESETS: { id: Preset; label: string }[] = [
    { id: "best", label: "Best" },
    { id: "1080p", label: "1080p" },
    { id: "720p", label: "720p" },
    { id: "audio", label: "Audio Only" },
];

/**
 * Result of resolving a preset against the current format list.
 *
 * - `"formatId"` — pick this single `format_id` and download it directly.
 * - `"selector"` — pass this DSL string to the backend's format selector
 *   (e.g. `"bv*+ba/best"` for auto-pair). Used when the best choice is a
 *   video-only + audio-only merge rather than any single row.
 */
type PresetResult =
    | { kind: "formatId"; value: string }
    | { kind: "selector"; value: string };

function findFormatForPreset(formats: FormatInfo[], preset: Preset): PresetResult | null {
    switch (preset) {
        case "best": {
            // Auto-pair bv+ba when BOTH a video-only and audio-only format
            // exist AND the video-only outranks the best muxed by height
            // (mirrors yt-dlp's default `bestvideo*+bestaudio/best`). Falls
            // back to the best muxed row otherwise — muxed-only sites keep
            // their original behaviour.
            const muxed = formats.filter((f) => f.has_video && f.has_audio);
            const videoOnly = formats.filter((f) => f.has_video && !f.has_audio);
            const audioOnly = formats.filter((f) => !f.has_video && f.has_audio);
            const bestMuxedHeight = Math.max(0, ...muxed.map((f) => f.height ?? 0));
            const bestVideoOnlyHeight = Math.max(0, ...videoOnly.map((f) => f.height ?? 0));
            const pairAvailable = videoOnly.length > 0 && audioOnly.length > 0;
            const pairBeatsMuxed = bestVideoOnlyHeight > bestMuxedHeight;
            if (pairAvailable && pairBeatsMuxed) {
                // `bv*` includes muxed as a video candidate, so on sites
                // where the muxed variant has the highest bitrate despite
                // lower resolution, the backend can still pick it. The
                // `/best` tail covers the degenerate case where `ba` fails.
                return { kind: "selector", value: "bv*+ba/best" };
            }
            const sorted = (muxed.length > 0 ? muxed : videoOnly).sort(
                (a, b) => (b.height ?? 0) - (a.height ?? 0),
            );
            const best = sorted[0]?.format_id;
            return best ? { kind: "formatId", value: best } : null;
        }
        case "1080p": {
            const candidates = formats.filter((f) => f.has_video && (f.height ?? 0) <= 1080);
            const top = candidates.sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0]
                ?.format_id;
            return top ? { kind: "formatId", value: top } : null;
        }
        case "720p": {
            const candidates = formats.filter((f) => f.has_video && (f.height ?? 0) <= 720);
            const top = candidates.sort((a, b) => (b.height ?? 0) - (a.height ?? 0))[0]
                ?.format_id;
            return top ? { kind: "formatId", value: top } : null;
        }
        case "audio": {
            const id = formats.filter((f) => !f.has_video && f.has_audio)[0]?.format_id;
            return id ? { kind: "formatId", value: id } : null;
        }
        default:
            return null;
    }
}

interface PresetToolbarProps {
    formats: FormatInfo[];
}

export function PresetToolbar({ formats }: PresetToolbarProps) {
    // `clickedPreset` captures the user's explicit preset choice. The
    // visible `activePreset` (lit pill) is derived below by checking
    // whether the current uiStore selection still matches that choice —
    // clicking a row manually via FormatsTable changes uiStore out from
    // under us, and the pill should deselect.
    const [clickedPreset, setClickedPreset] = useState<Preset | null>(null);
    const showExpertFormats = useStore(uiStore, (s) => s.showExpertFormats);
    const selectedFormatId = useStore(uiStore, (s) => s.selectedFormatId);
    const selectedSelector = useStore(uiStore, (s) => s.selectedSelector);

    // Count of video-only rows — used to decide whether the Expert toggle is
    // even meaningful on this particular video (most sites expose zero).
    const videoOnlyCount = formats.filter(
        (f) => f.has_video && !f.has_audio,
    ).length;

    // Visible preset: the clicked one IF its resolved value still matches
    // what's currently selected. Deriving at render time (rather than via
    // useEffect) per the project's React-state rules — the value has no
    // independent identity, it's a pure function of (clickedPreset, formats,
    // uiStore selection).
    const activePreset: Preset | null = useMemo(() => {
        if (!clickedPreset) return null;
        const result = findFormatForPreset(formats, clickedPreset);
        if (!result) return null;
        const matches = result.kind === "selector"
            ? selectedSelector === result.value
            : selectedFormatId === result.value;
        return matches ? clickedPreset : null;
    }, [clickedPreset, formats, selectedFormatId, selectedSelector]);

    function handlePreset(preset: Preset) {
        setClickedPreset(preset);
        const result = findFormatForPreset(formats, preset);
        if (!result) return;
        if (result.kind === "selector") {
            setSelectedSelector(result.value);
        } else {
            setSelectedFormat(result.value);
        }
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

            {videoOnlyCount > 0 && (
                <Checkbox
                    isSelected={showExpertFormats}
                    onChange={setShowExpertFormats}
                    className="flex items-center gap-1.5 text-[10px] text-[#666666] hover:text-[#aaaaaa] cursor-pointer select-none"
                    aria-label="Show video-only streams"
                >
                    {({ isSelected }) => (
                        <>
                            <span
                                className={cn(
                                    "inline-block w-[11px] h-[11px] rounded-[2px] border",
                                    isSelected
                                        ? "bg-[#4a9eff] border-[#4a9eff]"
                                        : "bg-transparent border-[#444444]",
                                )}
                            />
                            <span>Video-only streams ({videoOnlyCount})</span>
                        </>
                    )}
                </Checkbox>
            )}

            <span className="text-[10px] text-[#444444]">
                {formats.length} {formats.length === 1 ? "format" : "formats"}
            </span>
        </div>
    );
}
