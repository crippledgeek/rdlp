import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { settingsQueryOptions, updateSettings } from "../api/settings";
import { providersQueryOptions } from "../api/search";
import type { AppSettings } from "../types";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Separator } from "@/components/ui/separator";
import { OutputSection } from "./settings/OutputSection";
import { FormatDefaultsSection } from "./settings/FormatDefaultsSection";
import { MediaSection } from "./settings/MediaSection";
import { SearchSection } from "./settings/SearchSection";
import { CookiesSection } from "./settings/CookiesSection";
import { NetworkSection } from "./settings/NetworkSection";
import { AudioNormSection } from "./settings/AudioNormSection";

export function SettingsPage() {
    const { data: settings, isLoading: settingsLoading } = useQuery(settingsQueryOptions());
    const { data: providers = [] } = useQuery(providersQueryOptions());
    const [draft, setDraft] = useState<AppSettings | null>(null);
    const [saveError, setSaveError] = useState<string | null>(null);

    useEffect(() => {
        if (settings) {
            setDraft(settings);
        }
    }, [settings]);

    if (settingsLoading || !draft) {
        return <div className="flex items-center justify-center py-16 text-muted-foreground">Loading settings...</div>;
    }

    const handleChange = (next: AppSettings) => setDraft(next);

    const handleSave = async () => {
        // Range validation for loudnorm numeric fields
        if (draft.loudnorm_target_i !== null && (draft.loudnorm_target_i < -70 || draft.loudnorm_target_i > 0)) {
            setSaveError("Loudness Target must be between -70 and 0 LUFS.");
            return;
        }
        if (draft.loudnorm_target_tp !== null && (draft.loudnorm_target_tp < -9 || draft.loudnorm_target_tp > 0)) {
            setSaveError("True Peak Limit must be between -9 and 0 dBTP.");
            return;
        }
        if (draft.loudnorm_target_lra !== null && (draft.loudnorm_target_lra < 1 || draft.loudnorm_target_lra > 30)) {
            setSaveError("Loudness Range must be between 1 and 30 LU.");
            return;
        }
        if (draft.normalize_boost_db !== null && (draft.normalize_boost_db < 0 || draft.normalize_boost_db > 30)) {
            setSaveError("Boost Gain must be between 0 and 30 dB.");
            return;
        }
        try {
            setSaveError(null);
            await updateSettings(draft);
        } catch (err: unknown) {
            const msg =
                err instanceof Error
                    ? err.message
                    : typeof err === "object" && err !== null && "message" in err && typeof (err as Record<string, unknown>).message === "string"
                      ? (err as Record<string, string>).message
                      : String(err);
            setSaveError(msg);
        }
    };

    return (
        <div className="max-w-xl">
            <h2 className="text-lg font-bold mb-4 text-foreground">Settings</h2>

            <OutputSection draft={draft} onChange={handleChange} />

            <Separator className="my-6" />

            <FormatDefaultsSection draft={draft} onChange={handleChange} />

            <MediaSection draft={draft} onChange={handleChange} />

            <SearchSection draft={draft} onChange={handleChange} providers={providers} />

            <Separator className="my-6" />

            <CookiesSection draft={draft} onChange={handleChange} />

            <Separator className="my-6" />

            <NetworkSection draft={draft} onChange={handleChange} />

            <Separator className="my-6" />

            <AudioNormSection draft={draft} onChange={handleChange} />

            {saveError && (
                <Alert variant="destructive" className="my-4">
                    <AlertDescription>{saveError}</AlertDescription>
                </Alert>
            )}

            <Button onClick={handleSave} className="bg-primary text-primary-foreground hover:bg-primary/90">
                Save Settings
            </Button>
        </div>
    );
}
