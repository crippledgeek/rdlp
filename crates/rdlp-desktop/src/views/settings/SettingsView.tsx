// SettingsView: full-width settings form with section-based layout.
// ConfigPanel is hidden when this view is active (settings uses full width).

import { useState } from "react";
import { extractErrorMessage } from "@/api/invokeClient";
import { useQuery } from "@tanstack/react-query";
import { settingsQueryOptions, updateSettings } from "@/api/settings";
import { builtinNetworkDefaultsQueryOptions, effectiveNetworkQueryOptions } from "@/api/effectiveNetwork";
import { effectiveNormalizeQueryOptions, loudnormPresetsQueryOptions } from "@/api/effectiveNormalize";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { GeneralSection } from "./sections/GeneralSection";
import { OutputSection } from "./sections/OutputSection";
import { PostProcessSection } from "./sections/PostProcessSection";
import { DownloadSection } from "./sections/DownloadSection";
import { SubtitlesSection } from "./sections/SubtitlesSection";
import { NormalizationSection } from "./sections/NormalizationSection";
import { NetworkSection } from "./sections/NetworkSection";
import { SystemSection } from "./sections/SystemSection";
import type { AppSettings } from "@/types";

export function SettingsView() {
    const { data: settings, error: settingsError } = useQuery(settingsQueryOptions());
    // The values an empty (inherit) field resolves to — the sections derive
    // their placeholders from this instead of carrying a copy of the defaults
    // (#611). Fetched in parallel with the settings; both gate the form.
    const { data: defaults, error: defaultsError } = useQuery(effectiveNetworkQueryOptions());
    // The built-in defaults, for the one control whose inherited value can be
    // a sentinel that cannot express "on" (idle eviction over an inherited 0).
    const { data: builtin, error: builtinError } = useQuery(builtinNetworkDefaultsQueryOptions());
    // Track edits as a partial overlay on top of server data.
    // null = no edits yet, show server data as-is.
    const [edits, setEdits] = useState<Partial<AppSettings> | null>(null);
    const [saveError, setSaveError] = useState<string | null>(null);
    const [saved, setSaved] = useState(false);

    // Computed draft: server data merged with local edits
    const draft = settings ? { ...settings, ...edits } : null;

    // The normalization values for the DRAFT'S preset — re-fetched when the
    // preset changes, because the I/TP/LRA defaults are preset-dependent
    // (#611). `keepPreviousData` in the options holds the last payload while
    // the next loads, so a preset switch never blanks the placeholders.
    const { data: normalize, error: normalizeError } = useQuery(
        effectiveNormalizeQueryOptions(draft?.loudnorm_preset ?? null),
    );

    // The preset catalogue for the picker's per-item labels (#611).
    const { data: presets, error: presetsError } = useQuery(loudnormPresetsQueryOptions());

    const loadError = settingsError ?? defaultsError ?? builtinError ?? normalizeError ?? presetsError;
    if (loadError) {
        return (
            <div className="max-w-2xl mx-auto px-4 py-6">
                <Alert variant="destructive">
                    <AlertDescription>
                        Failed to load settings: {extractErrorMessage(loadError)}
                    </AlertDescription>
                </Alert>
            </div>
        );
    }

    if (!draft || !defaults || !builtin || !normalize || !presets) {
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

    // No client-side range table: the ranges have one owner
    // (`rdlp_types::EffectiveNormalize::*_RANGE` etc.), enforced by
    // `AppSettings::validate_security` behind `update_settings`, and its
    // `OutOfRange` verdict surfaces through the Alert below (#611 review).
    const handleSave = async () => {
        if (!draft) return;
        try {
            setSaveError(null);
            await updateSettings(draft);
            setEdits(null); // Clear edits — server data is now the source of truth
            setSaved(true);
            setTimeout(() => setSaved(false), 2000);
        } catch (e: unknown) {
            setSaveError(extractErrorMessage(e) || "Failed to save settings");
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
                <DownloadSection draft={draft} defaults={defaults} onChange={handleChange} />
                <SubtitlesSection draft={draft} onChange={handleChange} />
                <NormalizationSection draft={draft} effective={normalize} presets={presets} onChange={handleChange} />
                <NetworkSection draft={draft} defaults={defaults} builtin={builtin} onChange={handleChange} />
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
