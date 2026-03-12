import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import type { SettingsSectionProps } from "./types";

const NONE_SENTINEL = "none";

export function CookiesSection({ draft, onChange }: SettingsSectionProps) {
    return (
        <>
            <h3
                id="cookies-heading"
                className="text-sm font-bold text-foreground mb-3"
            >
                Cookies
            </h3>

            <section aria-labelledby="cookies-heading">
                <div className="mb-4">
                    <Label className="settings-label">Browser</Label>
                    <Select
                        value={draft.cookies_from_browser ?? NONE_SENTINEL}
                        onValueChange={(val) =>
                            onChange({
                                ...draft,
                                cookies_from_browser: val === NONE_SENTINEL ? null : val,
                            })
                        }
                    >
                        <SelectTrigger className={cn("w-full text-sm", draft.cookies_from_browser && "select-active")}>
                            <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                            <SelectItem value="chrome">Chrome</SelectItem>
                            <SelectItem value="firefox">Firefox</SelectItem>
                        </SelectContent>
                    </Select>
                </div>

                <div className="mb-4">
                    <Label htmlFor="cookies-file" className="settings-label">
                        Cookie File (Netscape format)
                    </Label>
                    <Input
                        id="cookies-file"
                        type="text"
                        placeholder="/path/to/cookies.txt"
                        value={draft.cookies_file ?? ""}
                        onChange={(e) =>
                            onChange({ ...draft, cookies_file: e.target.value || null })
                        }
                        className="font-mono text-xs"
                    />
                </div>
            </section>
        </>
    );
}
