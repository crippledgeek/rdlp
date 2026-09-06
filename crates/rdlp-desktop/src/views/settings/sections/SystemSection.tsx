// SystemSection: read-only system info (FFmpeg encoders, codecs).

import { Cpu } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { codecsQueryOptions } from "@/api/codecs";

export function SystemSection() {
    // `?? []`, not a `= []` destructuring default: the default fires only on
    // undefined. The declared contract cannot produce null — the Rust command
    // returns `Result<Vec<_>, AppError>` and `invokeTyped<T>` resolves T or
    // throws — so this is hardening rather than a fix for a reachable bug. It
    // exists because a null DID reach here once, fabricated by the Cypress
    // stub's fallback for unregistered commands, and crashed this whole
    // section. Cheap insurance against any future source of the same shape.
    const { data } = useQuery(codecsQueryOptions(true));
    const codecs = data ?? [];

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
