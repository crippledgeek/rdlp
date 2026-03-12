import { cn } from "@/lib/utils";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import type { SearchSiteInfo } from "../../types";
import type { SettingsSectionProps } from "./types";

const NONE_SENTINEL = "none";

interface SearchSectionProps extends SettingsSectionProps {
    providers: SearchSiteInfo[];
}

export function SearchSection({ draft, onChange, providers }: SearchSectionProps) {
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
}
