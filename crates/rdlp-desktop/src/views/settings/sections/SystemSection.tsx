// SystemSection: read-only system info (FFmpeg encoders, codecs).

import { Cpu } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { codecsQueryOptions } from "@/api/codecs";

export function SystemSection() {
    const { data: codecs = [] } = useQuery(codecsQueryOptions());

    if (codecs.length === 0) return null;

    const encoderNames = codecs.flatMap((c) => c.encoders.map((e) => e.encoderName));

    return (
        <section id="settings-system" aria-labelledby="settings-system-heading" className="settings-panel">
            <h3 id="settings-system-heading" className="settings-panel-title">
                <Cpu className="size-3.5" />
                System Info
            </h3>
            <div>
                <p className="text-xs text-muted-foreground leading-relaxed">
                    <span className="font-medium text-foreground">FFmpeg encoders: </span>
                    {encoderNames.join(", ")}
                </p>
            </div>
        </section>
    );
}
