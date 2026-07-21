// SubtitlesSection: subtitle download/verification toggles.

import { Captions } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import type { AppSettings } from "@/types";

interface Props {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

// Jolly Checkbox (React Aria) uses isSelected + children for label (no separate Label + id pairing)
const toggleItems: { label: string; field: keyof AppSettings; help: string }[] = [
    { label: "Download subtitles", field: "write_subtitles", help: "Fetch subtitle tracks for each download." },
    {
        label: "Download auto-generated subtitles",
        field: "write_auto_subtitles",
        help: "Include machine-generated captions when available.",
    },
    {
        label: "Strict subtitles",
        field: "strict_subs",
        help: "Fail the download if requested subtitles are missing.",
    },
    {
        label: "Verify subtitle URLs",
        field: "verify_sub_urls",
        help: "Check subtitle URLs are reachable before downloading.",
    },
    { label: "Retry failed subtitles", field: "retry_subs", help: "Retry subtitle downloads that fail." },
];

export function SubtitlesSection({ draft, onChange }: Props) {
    return (
        <section id="settings-subtitles" aria-labelledby="settings-subtitles-heading" className="settings-panel">
            <h3 id="settings-subtitles-heading" className="settings-panel-title">
                <Captions className="size-3.5" />
                Subtitles
            </h3>
            <div className="flex flex-col gap-3">
                {toggleItems.map(({ label, field, help }) => (
                    <div key={field} className="flex flex-col gap-1">
                        <Checkbox
                            isSelected={draft[field] as boolean}
                            onChange={(checked) => onChange({ [field]: checked })}
                            className="flex items-center gap-2 text-sm font-medium text-muted-foreground cursor-pointer"
                        >
                            {label}
                        </Checkbox>
                        <p className="text-xs text-muted-foreground/70 pl-6">{help}</p>
                    </div>
                ))}
            </div>
        </section>
    );
}
