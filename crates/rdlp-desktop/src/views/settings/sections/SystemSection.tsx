// SystemSection: read-only system info (FFmpeg encoders, codecs).

import { Cpu } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { codecsQueryOptions } from "@/api/codecs";

export function SystemSection() {
    // `?? []`, not a `= []` destructuring default: the default fires only on
    // undefined. The contract cannot produce null — `available_codecs` is
    // INFALLIBLE, returning a plain `Vec<VideoCodecInfo>`
    // (src-tauri/src/commands/codecs.rs:15), so not even an error path yields
    // one. This is hardening, not a fix for a reachable bug.
    //
    // It exists because a null DID reach here once, fabricated by the Cypress
    // stub's fallback for unregistered commands, and crashed this whole
    // section. Note what let it through: `invokeTyped<T>` DECLARES `T` but
    // never validates it — api/invokeClient.ts:59 is a bare pass-through — so
    // the type was a claim rather than a guarantee. That is the reason for the
    // guard, not merely the reason the bug happened.
    //
    // Untested by construction: the stub now returns codecs, so nothing in the
    // suite can drive a null into this line.
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
