import { memo } from "react";
import { cn } from "@/lib/utils";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { NONE_SENTINEL } from "./utils/formatConstants";
import type { AppSettings, SearchSiteInfo } from "../types";

interface SettingsSearchSectionProps {
    draft: AppSettings;
    onChange: (next: AppSettings) => void;
    providers: SearchSiteInfo[];
}

export const SettingsSearchSection = memo(function SettingsSearchSection({
    draft,
    onChange,
    providers,
}: SettingsSearchSectionProps) {
    return (
        <div className="mb-4">
            <Label className="settings-label">Default Search Provider</Label>
            <Select
                value={draft.default_search_provider ?? NONE_SENTINEL}
                onValueChange={(val) =>
                    onChange({
                        ...draft,
                        default_search_provider: val === NONE_SENTINEL ? null : val,
                    })
                }
            >
                <SelectTrigger className={cn("w-full text-sm", draft.default_search_provider && "select-active")}>
                    <SelectValue />
                </SelectTrigger>
                <SelectContent>
                    <SelectItem value={NONE_SENTINEL}>Auto</SelectItem>
                    {providers.map((p) => (
                        <SelectItem key={p.name} value={p.name}>
                            {p.display_name}
                        </SelectItem>
                    ))}
                </SelectContent>
            </Select>
        </div>
    );
});
