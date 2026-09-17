// NetworkSection: proxy, rate limit, timeouts, and cookie settings.

import { Globe, KeyRound } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectTrigger, SelectValue, SelectItem, SelectPopover, SelectListBox } from "@/components/ui/select";
import { NumericField } from "@/views/settings/NumericField";
import { FormDescription } from "@/components/ui/field";
import {
    formStateToPoolIdleTimeout,
    poolIdleTimeoutToFormState,
    type PoolIdleFormState,
} from "@/views/settings/networkSchema";
import { withInheritHint } from "@/views/settings/inheritHint";
import type { AppSettings, EffectiveNetwork } from "@/types";

const NONE_KEY = "none";

/** Bounds of the idle-timeout control; `0` is never typeable here — it is the checkbox's sentinel. */
const POOL_IDLE_MIN_SECS = 1;
const POOL_IDLE_MAX_SECS = 3600;

interface Props {
    draft: AppSettings;
    /**
     * The values the engine runs with when a field is left empty (inherit),
     * served over IPC. Every placeholder below derives from it — none is a
     * literal (#611; enforced by `scripts/check-effective-network-drift.sh`).
     */
    defaults: EffectiveNetwork;
    onChange: (update: Partial<AppSettings>) => void;
}

export function NetworkSection({ draft, defaults, onChange }: Props) {
    // A null draft INHERITS the base configuration, which may itself be the
    // 0-sentinel ("eviction disabled"): then the checkbox must show OFF and the
    // numeric input must not advertise a "0" placeholder below its own minValue.
    const inheritsEvictionOff = draft.pool_idle_timeout === null && defaults.pool_idle_timeout_secs === 0;
    const poolIdleForm: PoolIdleFormState = inheritsEvictionOff
        ? { evictIdle: false, secondsInput: "" }
        : poolIdleTimeoutToFormState(draft.pool_idle_timeout);
    // NumericField already owns the in-progress-text vs committed-number split
    // and clamps to [minValue, maxValue] before `onCommit` fires (see
    // NumericField.tsx). The 0-sentinel ("disable eviction") stays owned by
    // the checkbox — NumericField's own minValue=1 means the numeric control
    // itself can never produce 0.
    const handleEvictToggle = (next: boolean) => {
        // Turning eviction ON while inheriting the 0-sentinel cannot be
        // expressed as "inherit" (that IS off), so it needs an explicit
        // positive value: seed the control's own lower bound for the user to
        // edit. Every other transition keeps the existing form-state mapping.
        if (next && inheritsEvictionOff) {
            onChange({ pool_idle_timeout: POOL_IDLE_MIN_SECS });
            return;
        }
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
                        helper={withInheritHint("Time to establish a connection to the server.")}
                        value={draft.socket_timeout}
                        minValue={1}
                        maxValue={300}
                        onCommit={(v) => onChange({ socket_timeout: v })}
                        placeholder={String(defaults.socket_timeout_secs)}
                        suffix="s"
                    />
                    <NumericField
                        id="read-timeout"
                        label="Read Timeout"
                        helper={withInheritHint("Maximum gap between bytes during a download.")}
                        value={draft.read_timeout}
                        minValue={1}
                        maxValue={600}
                        onCommit={(v) => onChange({ read_timeout: v })}
                        placeholder={String(defaults.read_timeout_secs)}
                        suffix="s"
                    />
                    <NumericField
                        id="download-timeout"
                        label="Download Timeout"
                        helper={withInheritHint("Maximum time for the entire file download.")}
                        value={draft.download_timeout}
                        minValue={1}
                        maxValue={86400}
                        onCommit={(v) => onChange({ download_timeout: v })}
                        placeholder={String(defaults.download_timeout_secs)}
                        suffix="s"
                    />
                    <NumericField
                        id="merge-timeout"
                        label="Merge Timeout"
                        helper={withInheritHint("Maximum time to mux/merge the downloaded parts.")}
                        value={draft.merge_timeout}
                        minValue={1}
                        maxValue={86400}
                        onCommit={(v) => onChange({ merge_timeout: v })}
                        placeholder={String(defaults.merge_timeout_secs)}
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
                                    // Helper text moved to the full-width FormDescription sibling below
                                    // (id="pool-idle-timeout-description") — this narrow (96px) column
                                    // would otherwise wrap it awkwardly. Wired back to the field via
                                    // aria-describedby so it stays part of the accessible description.
                                    helper=""
                                    aria-describedby="pool-idle-timeout-description"
                                    hideLabel
                                    value={poolIdleForm.evictIdle && poolIdleForm.secondsInput !== "" ? Number(poolIdleForm.secondsInput) : null}
                                    minValue={POOL_IDLE_MIN_SECS}
                                    maxValue={POOL_IDLE_MAX_SECS}
                                    onCommit={handlePoolIdleChange}
                                    isDisabled={!poolIdleForm.evictIdle}
                                    // No placeholder when the inherited value is the 0-sentinel: "0" would
                                    // sit below minValue and claim a timeout that does not exist.
                                    {...(defaults.pool_idle_timeout_secs !== 0
                                        ? { placeholder: String(defaults.pool_idle_timeout_secs) }
                                        : {})}
                                    suffix="s"
                                />
                            </div>
                        </div>
                        <FormDescription id="pool-idle-timeout-description" className="mt-1">
                            {withInheritHint(
                                "When off, idle keep-alive connections are kept until the OS closes them.",
                            )}
                        </FormDescription>
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
