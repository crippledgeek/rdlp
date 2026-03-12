import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import {
    Tooltip,
    TooltipContent,
    TooltipProvider,
    TooltipTrigger,
} from "@/components/ui/tooltip";
import { pickDirectory } from "../../api/settings";
import type { SettingsSectionProps } from "./types";

export function OutputSection({ draft, onChange }: SettingsSectionProps) {
    const handlePickDir = async () => {
        const dir = await pickDirectory();
        if (dir) {
            onChange({ ...draft, output_dir: dir });
        }
    };

    return (
        <>
            <div className="mb-4">
                <Label className="settings-label">Output Directory</Label>
                <div className="flex gap-1.5">
                    <Input type="text" value={draft.output_dir} readOnly className="flex-1 font-mono text-xs" />
                    <Button variant="outline" onClick={handlePickDir}>Browse</Button>
                </div>
            </div>

            <div className="mb-4">
                <div className="flex items-center gap-1.5 mb-1">
                    <Label htmlFor="output-template" className="settings-label">Output Filename Template</Label>
                    <TooltipProvider>
                        <Tooltip>
                            <TooltipTrigger asChild>
                                <span className="text-xs text-muted-foreground cursor-help underline decoration-dotted">
                                    ?
                                </span>
                            </TooltipTrigger>
                            <TooltipContent side="right" className="max-w-xs text-xs">
                                <p className="font-semibold mb-1">Common variables:</p>
                                <ul className="space-y-0.5">
                                    <li><code>%(title)s</code> — Video title</li>
                                    <li><code>%(ext)s</code> — File extension</li>
                                    <li><code>%(uploader)s</code> — Uploader name</li>
                                    <li><code>%(upload_date)s</code> — Upload date (YYYYMMDD)</li>
                                    <li><code>%(id)s</code> — Video ID</li>
                                    <li><code>%(playlist_index)s</code> — Playlist position</li>
                                </ul>
                                <p className="mt-1 text-muted-foreground">e.g. <code>%(uploader)s/%(title)s.%(ext)s</code></p>
                            </TooltipContent>
                        </Tooltip>
                    </TooltipProvider>
                </div>
                <Input
                    id="output-template"
                    type="text"
                    placeholder="%(title)s.%(ext)s"
                    value={draft.output_template ?? ""}
                    onChange={(e) =>
                        onChange({ ...draft, output_template: e.target.value || null })
                    }
                    className="font-mono text-xs"
                />
            </div>
        </>
    );
}
