import { cn } from "@/lib/utils";
import { Label } from "@/components/ui/label";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import type { AudioFormat, ContainerFormat } from "../../types";
import type { SettingsSectionProps } from "./types";

const NONE_SENTINEL = "none";

export function FormatDefaultsSection({ draft, onChange }: SettingsSectionProps) {
    return (
        <>
            <div className="mb-4">
                <Label className="settings-label">Default Remux Format</Label>
                <Select
                    value={draft.default_remux ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        onChange({
                            ...draft,
                            default_remux: val === NONE_SENTINEL ? null : (val as ContainerFormat),
                        })
                    }
                >
                    <SelectTrigger className={cn("w-full text-sm", draft.default_remux && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                        <SelectItem value="mp4">MP4</SelectItem>
                        <SelectItem value="mkv">MKV</SelectItem>
                        <SelectItem value="webm">WebM</SelectItem>
                    </SelectContent>
                </Select>
            </div>

            <div className="mb-4">
                <Label className="settings-label">Default Audio Extraction</Label>
                <Select
                    value={draft.default_extract_audio ?? NONE_SENTINEL}
                    onValueChange={(val) =>
                        onChange({
                            ...draft,
                            default_extract_audio: val === NONE_SENTINEL ? null : (val as AudioFormat),
                        })
                    }
                >
                    <SelectTrigger className={cn("w-full text-sm", draft.default_extract_audio && "select-active")}>
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value={NONE_SENTINEL}>None</SelectItem>
                        <SelectItem value="mp3">MP3</SelectItem>
                        <SelectItem value="aac">AAC</SelectItem>
                        <SelectItem value="opus">Opus</SelectItem>
                        <SelectItem value="flac">FLAC</SelectItem>
                    </SelectContent>
                </Select>
            </div>
        </>
    );
}
