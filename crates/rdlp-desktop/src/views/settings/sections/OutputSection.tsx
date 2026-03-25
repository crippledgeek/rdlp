// OutputSection: output directory and filename template settings.

import { FolderOpen } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipTrigger } from "@/components/ui/tooltip";
import { pickDirectory } from "@/api/settings";
import type { AppSettings } from "@/types";

const TEMPLATE_VARS = `%(title)s — Video title\n%(ext)s — File extension\n%(uploader)s — Uploader name\n%(upload_date)s — Upload date\n%(id)s — Video ID`;

interface Props {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

export function OutputSection({ draft, onChange }: Props) {
    const handlePickDir = async () => {
        const dir = await pickDirectory();
        if (dir) onChange({ output_dir: dir });
    };

    return (
        <section id="settings-output" aria-labelledby="settings-output-heading" className="settings-panel">
            <h3 id="settings-output-heading" className="settings-panel-title">
                <FolderOpen className="size-3.5" />
                Output &amp; Templates
            </h3>
            <div className="space-y-3">
                <div>
                    <Label className="settings-label">Output Directory</Label>
                    <div className="flex gap-1.5">
                        <Input
                            type="text"
                            value={draft.output_dir}
                            readOnly
                            className="flex-1 font-mono text-xs"
                        />
                        <Button variant="outline" onPress={() => { void handlePickDir(); }}>
                            Browse
                        </Button>
                    </div>
                </div>
                <div>
                    <div className="flex items-center gap-1.5 mb-1">
                        <Label htmlFor="output-template" className="settings-label">
                            Filename Template
                        </Label>
                        <TooltipTrigger delay={200}>
                            <button
                                aria-label="Template variables help"
                                className="text-xs text-muted-foreground cursor-help underline decoration-dotted"
                            >
                                ?
                            </button>
                            <Tooltip>
                                <div className="text-xs space-y-0.5">
                                    <p className="font-semibold mb-1">Common variables:</p>
                                    {TEMPLATE_VARS.split("\n").map((line) => (
                                        <p key={line}><code>{line.split(" — ")[0]}</code> — {line.split(" — ")[1]}</p>
                                    ))}
                                    <p className="mt-1 text-muted-foreground">e.g. <code>%(uploader)s/%(title)s.%(ext)s</code></p>
                                </div>
                            </Tooltip>
                        </TooltipTrigger>
                    </div>
                    <Input
                        id="output-template"
                        type="text"
                        placeholder="%(title)s.%(ext)s"
                        value={draft.output_template ?? ""}
                        onChange={(e) => onChange({ output_template: e.target.value || null })}
                        className="font-mono text-xs"
                    />
                </div>
            </div>
        </section>
    );
}
