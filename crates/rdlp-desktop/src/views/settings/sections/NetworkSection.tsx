// NetworkSection: proxy, rate limit, timeouts, and cookie settings.

import { Globe, KeyRound } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectTrigger, SelectValue, SelectItem, SelectPopover, SelectListBox } from "@/components/ui/select";
import { NumericField } from "@/views/settings/NumericField";
import {
    formStateToPoolIdleTimeout,
    poolIdleTimeoutToFormState,
    type PoolIdleFormState,
} from "@/views/settings/networkSchema";
import type { AppSettings } from "@/types";

const NONE_KEY = "none";

interface Props {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

export function NetworkSection({ draft, onChange }: Props) {
    const poolIdleForm: PoolIdleFormState = poolIdleTimeoutToFormState(draft.pool_idle_timeout);
    // NumericField already owns the in-progress-text vs committed-number split
    // and clamps to [minValue, maxValue] before `onCommit` fires (see
    // NumericField.tsx). The 0-sentinel ("disable eviction") stays owned by
    // the checkbox — NumericField's own minValue=1 means the numeric control
    // itself can never produce 0.
    const handleEvictToggle = (next: boolean) => {
        onChange({
            pool_idle_timeout: formStateToPoolIdleTimeout({
                evictIdle: next,
                secondsInput: poolIdleForm.secondsInput,
            }),
        });
    };

    const handlePoolIdleChange = (next: number | null) => {
        onChange({
            pool_idle_timeout: formStateToPoolIdleTimeout({
                evictIdle: poolIdleForm.evictIdle,
                secondsInput: next === null ? "" : String(next),
            }),
        });
    };

    return (
        <>
            {/* Network */}
            <section id="settings-network" aria-labelledby="settings-network-heading" className="settings-panel">
                <h3 id="settings-network-heading" className="settings-panel-title">
                    <Globe className="size-3.5" />
                    Network
                </h3>
                <div className="grid grid-cols-2 gap-x-4 gap-y-3">
                    <div>
                        <Label htmlFor="proxy" className="settings-label">Proxy</Label>
                        <Input
                            id="proxy"
                            type="text"
                            placeholder="http://proxy:8080"
                            value={draft.proxy ?? ""}
                            onChange={(e) => onChange({ proxy: e.target.value || null })}
                            className="font-mono text-xs"
                        />
                    </div>
                    <div>
                        <Label htmlFor="rate-limit" className="settings-label">Rate Limit</Label>
                        <Input
                            id="rate-limit"
                            type="text"
                            placeholder="500K, 2M"
                            value={draft.rate_limit ?? ""}
                            onChange={(e) => onChange({ rate_limit: e.target.value || null })}
                            className="font-mono text-xs"
                        />
                    </div>
                    <NumericField
                        id="socket-timeout"
                        label="Connection Timeout"
                        helper="Time to establish a connection to the server."
                        value={draft.socket_timeout}
                        minValue={1}
                        maxValue={300}
                        onCommit={(v) => onChange({ socket_timeout: v })}
                        placeholder="30"
                        suffix="s"
                    />
                    <NumericField
                        id="read-timeout"
                        label="Read Timeout"
                        helper="Maximum gap between bytes during a download."
                        value={draft.read_timeout}
                        minValue={1}
                        maxValue={600}
                        onCommit={(v) => onChange({ read_timeout: v })}
                        placeholder="60"
                        suffix="s"
                    />
                    <NumericField
                        id="download-timeout"
                        label="Download Timeout"
                        helper="Maximum time for the entire file download."
                        value={draft.download_timeout}
                        minValue={1}
                        maxValue={86400}
                        onCommit={(v) => onChange({ download_timeout: v })}
                        placeholder="3600"
                        suffix="s"
                    />
                    <NumericField
                        id="merge-timeout"
                        label="Merge Timeout"
                        helper="Maximum time to mux/merge the downloaded parts."
                        value={draft.merge_timeout}
                        minValue={1}
                        maxValue={86400}
                        onCommit={(v) => onChange({ merge_timeout: v })}
                        placeholder="1800"
                        suffix="s"
                    />
                    <div className="col-span-2">
                        <div className="flex items-center gap-2 flex-wrap">
                            <Checkbox
                                id="evict-idle"
                                isSelected={poolIdleForm.evictIdle}
                                onChange={handleEvictToggle}
                                aria-controls="pool-idle-timeout"
                            >
                                <span className="settings-label !mb-0">Evict idle connections after</span>
                            </Checkbox>
                            <div className="w-24">
                                <NumericField
                                    id="pool-idle-timeout"
                                    label="Idle Timeout"
                                    helper="When off, idle keep-alive connections are kept until the OS closes them."
                                    value={poolIdleForm.evictIdle && poolIdleForm.secondsInput !== "" ? Number(poolIdleForm.secondsInput) : null}
                                    minValue={1}
                                    maxValue={3600}
                                    onCommit={handlePoolIdleChange}
                                    isDisabled={!poolIdleForm.evictIdle}
                                    placeholder="90"
                                    suffix="s"
                                />
                            </div>
                        </div>
                    </div>
                </div>
            </section>

            {/* Cookies */}
            <section id="settings-cookies" aria-labelledby="settings-cookies-heading" className="settings-panel">
                <h3 id="settings-cookies-heading" className="settings-panel-title">
                    <KeyRound className="size-3.5" />
                    Cookies
                </h3>
                <div className="grid grid-cols-2 gap-x-4 gap-y-3">
                    <div>
                        <Label className="settings-label">Browser</Label>
                        <Select
                            selectedKey={draft.cookies_from_browser ?? NONE_KEY}
                            onSelectionChange={(key) => {
                                const k = String(key);
                                onChange({ cookies_from_browser: k === NONE_KEY ? null : k });
                            }}
                        >
                            <SelectTrigger className="w-full text-sm">
                                <SelectValue />
                            </SelectTrigger>
                            <SelectPopover>
                                <SelectListBox>
                                    <SelectItem id={NONE_KEY}>None</SelectItem>
                                    <SelectItem id="chrome">Chrome</SelectItem>
                                    <SelectItem id="firefox">Firefox</SelectItem>
                                </SelectListBox>
                            </SelectPopover>
                        </Select>
                    </div>
                    <div>
                        <Label htmlFor="cookies-file" className="settings-label">
                            Cookie File (Netscape)
                        </Label>
                        <Input
                            id="cookies-file"
                            type="text"
                            placeholder="/path/to/cookies.txt"
                            value={draft.cookies_file ?? ""}
                            onChange={(e) => onChange({ cookies_file: e.target.value || null })}
                            className="font-mono text-xs"
                        />
                    </div>
                </div>
            </section>
        </>
    );
}
