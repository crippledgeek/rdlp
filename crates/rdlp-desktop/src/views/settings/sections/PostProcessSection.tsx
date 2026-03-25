// PostProcessSection: media embedding toggles.

import { Layers } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import type { AppSettings } from "@/types";

interface Props {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

// Jolly Checkbox (React Aria) uses isSelected + children for label (no separate Label + id pairing)
const toggleItems: { label: string; field: keyof AppSettings }[] = [
    { label: "Embed thumbnails", field: "embed_thumbnail" },
    { label: "Save thumbnail to disk", field: "write_thumbnail" },
    { label: "Embed metadata", field: "embed_metadata" },
    { label: "Embed subtitles", field: "embed_subtitles" },
    { label: "Verbose logging", field: "verbose" },
];

export function PostProcessSection({ draft, onChange }: Props) {
    return (
        <section id="settings-postprocess" aria-labelledby="settings-postprocess-heading" className="settings-panel">
            <h3 id="settings-postprocess-heading" className="settings-panel-title">
                <Layers className="size-3.5" />
                Media &amp; Embedding
            </h3>
            <div className="grid grid-cols-2 gap-2">
                {toggleItems.map(({ label, field }) => (
                    <Checkbox
                        key={field}
                        isSelected={draft[field] as boolean}
                        onChange={(checked) => onChange({ [field]: checked })}
                        className="flex items-center gap-2 text-sm font-medium text-muted-foreground cursor-pointer"
                    >
                        {label}
                    </Checkbox>
                ))}
            </div>
        </section>
    );
}
