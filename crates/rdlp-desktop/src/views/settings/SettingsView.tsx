// SettingsView: full-width settings form with section-based layout.
// ConfigPanel is hidden when this view is active (settings uses full width).

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { settingsQueryOptions, updateSettings } from "@/api/settings";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { GeneralSection } from "./sections/GeneralSection";
import { OutputSection } from "./sections/OutputSection";
import { PostProcessSection } from "./sections/PostProcessSection";
import { SubtitlesSection } from "./sections/SubtitlesSection";
import { NormalizationSection } from "./sections/NormalizationSection";
import { NetworkSection } from "./sections/NetworkSection";
import { SystemSection } from "./sections/SystemSection";
import type { AppSettings } from "@/types";

/** Validate settings before save. Returns error message or null if valid. */
function validateSettings(draft: AppSettings): string | null {
    if (
        draft.loudnorm_target_i !== null &&
        (draft.loudnorm_target_i < -70 || draft.loudnorm_target_i > 0)
    ) {
        return "Loudness Target must be between -70 and 0 LUFS.";
    }
    if (
        draft.loudnorm_target_tp !== null &&
        (draft.loudnorm_target_tp < -9 || draft.loudnorm_target_tp > 0)
    ) {
        return "True Peak Limit must be between -9 and 0 dBTP.";
    }
    if (
        draft.loudnorm_target_lra !== null &&
        (draft.loudnorm_target_lra < 1 || draft.loudnorm_target_lra > 30)
    ) {
        return "Loudness Range must be between 1 and 30 LU.";
    }
    if (
        draft.normalize_boost_db !== null &&
        (draft.normalize_boost_db < 0 || draft.normalize_boost_db > 30)
    ) {
        return "Boost Gain must be between 0 and 30 dB.";
    }
    return null;
}

export function SettingsView() {
    const { data: settings, isLoading } = useQuery(settingsQueryOptions());
    // Track edits as a partial overlay on top of server data.
    // null = no edits yet, show server data as-is.
    const [edits, setEdits] = useState<Partial<AppSettings> | null>(null);
    const [saveError, setSaveError] = useState<string | null>(null);
    const [saved, setSaved] = useState(false);

    // Computed draft: server data merged with local edits
    const draft = settings ? { ...settings, ...edits } : null;

    if (isLoading || !draft) {
        return (
            <div className="flex items-center justify-center h-full">
                <p className="text-[13px] text-[var(--text-muted)] animate-pulse">Loading settings…</p>
            </div>
        );
    }

    const handleChange = (update: Partial<AppSettings>) => {
        setEdits((prev) => ({ ...prev, ...update }));
        setSaved(false);
    };

    const handleSave = async () => {
        if (!draft) return;
        const err = validateSettings(draft);
        if (err) {
            setSaveError(err);
            return;
        }
        try {
            setSaveError(null);
            await updateSettings(draft);
            setEdits(null); // Clear edits — server data is now the source of truth
            setSaved(true);
            setTimeout(() => setSaved(false), 2000);
        } catch (e: unknown) {
            const msg =
                e instanceof Error
                    ? e.message
                    : typeof e === "object" && e !== null && "message" in e
                      ? String((e as Record<string, unknown>)["message"])
                      : String(e);
            setSaveError(msg);
        }
    };

    return (
        <div className="h-full overflow-y-auto">
            <div className="max-w-2xl mx-auto px-4 py-6 pb-16">
                <h2 className="text-[16px] font-semibold text-[#eeeeee] mb-6 tracking-tight">
                    Settings
                </h2>

                <GeneralSection draft={draft} onChange={handleChange} />
                <OutputSection draft={draft} onChange={handleChange} />
                <PostProcessSection draft={draft} onChange={handleChange} />
                <SubtitlesSection draft={draft} onChange={handleChange} />
                <NormalizationSection draft={draft} onChange={handleChange} />
                <NetworkSection draft={draft} onChange={handleChange} />
                <SystemSection />

                {/* Save */}
                {saveError && (
                    <Alert variant="destructive" className="mb-3">
                        <AlertDescription>{saveError}</AlertDescription>
                    </Alert>
                )}

                <div className="flex items-center gap-3">
                    <Button
                        onClick={() => { void handleSave(); }}
                        className="bg-[#4a9eff] text-white hover:bg-[#3a8eef]"
                    >
                        Save Settings
                    </Button>
                    {saved && (
                        <span className="text-[12px] text-[#4a9e4a]">Saved</span>
                    )}
                </div>
            </div>
        </div>
    );
}
