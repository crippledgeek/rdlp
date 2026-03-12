import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { SettingsSectionProps } from "./types";

export function NetworkSection({ draft, onChange }: SettingsSectionProps) {
    return (
        <>
            <h3
                id="network-heading"
                className="text-sm font-bold text-foreground mb-3"
            >
                Network
            </h3>

            <section aria-labelledby="network-heading">
                <div className="mb-4">
                    <Label htmlFor="proxy" className="settings-label">Proxy</Label>
                    <Input
                        id="proxy"
                        type="text"
                        placeholder="http://proxy:8080"
                        value={draft.proxy ?? ""}
                        onChange={(e) =>
                            onChange({ ...draft, proxy: e.target.value || null })
                        }
                        className="font-mono text-xs"
                    />
                </div>

                <div className="mb-4">
                    <Label htmlFor="rate-limit" className="settings-label">Rate Limit</Label>
                    <Input
                        id="rate-limit"
                        type="text"
                        placeholder="500K, 2M"
                        value={draft.rate_limit ?? ""}
                        onChange={(e) =>
                            onChange({ ...draft, rate_limit: e.target.value || null })
                        }
                        className="font-mono text-xs"
                    />
                </div>
            </section>
        </>
    );
}
