// NetworkSection: proxy, rate limit, timeouts, and cookie settings.

import { useState } from "react";
import { Globe, KeyRound } from "lucide-react";
import type { ZodTypeAny } from "zod";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectTrigger, SelectValue, SelectItem, SelectPopover, SelectListBox } from "@/components/ui/select";
import {
    socketTimeoutSchema,
    readTimeoutSchema,
    poolIdleTimeoutSchema,
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

interface TimeoutFieldProps {
    id: string;
    label: string;
    helper: string;
    initial: number | null;
    placeholder: string;
    schema: ZodTypeAny;
    onCommit: (next: number | null) => void;
    disabled?: boolean;
}

function TimeoutField({
    id,
    label,
    helper,
    initial,
    placeholder,
    schema,
    onCommit,
    disabled,
}: TimeoutFieldProps) {
    const [raw, setRaw] = useState<string>(initial === null ? "" : String(initial));
    const [error, setError] = useState<string | null>(null);

    const handleChange = (next: string) => {
        setRaw(next);
        const result = schema.safeParse(next);
        if (!result.success) {
            setError(result.error.errors[0]?.message ?? "Invalid value");
            return;
        }
        setError(null);
        onCommit(result.data as number | null);
    };

    return (
        <div>
            <Label htmlFor={id} className="settings-label">
                {label}
            </Label>
            <div className="flex items-center gap-1">
                <Input
                    id={id}
                    type="number"
                    inputMode="numeric"
                    min={0}
                    placeholder={placeholder}
                    value={raw}
                    onChange={(e) => handleChange(e.target.value)}
                    aria-describedby={`${id}-help`}
                    aria-invalid={error !== null}
                    disabled={disabled}
                    className="font-mono text-xs"
                />
                <span className="text-xs text-muted-foreground">s</span>
            </div>
            <p
                id={`${id}-help`}
                className={`text-xs mt-1 ${error ? "text-destructive" : "text-muted-foreground"}`}
            >
                {error ?? helper}
            </p>
        </div>
    );
}

export function NetworkSection({ draft, onChange }: Props) {
    const poolIdleForm: PoolIdleFormState = poolIdleTimeoutToFormState(draft.pool_idle_timeout);
    const [poolIdleRaw, setPoolIdleRaw] = useState<string>(poolIdleForm.secondsInput);
    const [poolIdleError, setPoolIdleError] = useState<string | null>(null);

    const handleEvictToggle = (next: boolean) => {
        onChange({
            pool_idle_timeout: formStateToPoolIdleTimeout({
                evictIdle: next,
                secondsInput: poolIdleRaw,
            }),
        });
    };

    const handlePoolIdleChange = (next: string) => {
        setPoolIdleRaw(next);
        const result = poolIdleTimeoutSchema.safeParse(next);
        if (!result.success) {
            setPoolIdleError(result.error.errors[0]?.message ?? "Invalid value");
            return;
        }
        setPoolIdleError(null);
        onChange({
            pool_idle_timeout: formStateToPoolIdleTimeout({
                evictIdle: poolIdleForm.evictIdle,
                secondsInput: next,
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
                    <TimeoutField
                        id="socket-timeout"
                        label="Connection Timeout"
                        helper="Time to establish a connection to the server."
                        initial={draft.socket_timeout}
                        placeholder="30"
                        schema={socketTimeoutSchema}
                        onCommit={(v) => onChange({ socket_timeout: v })}
                    />
                    <TimeoutField
                        id="read-timeout"
                        label="Read Timeout"
                        helper="Maximum gap between bytes during a download."
                        initial={draft.read_timeout}
                        placeholder="60"
                        schema={readTimeoutSchema}
                        onCommit={(v) => onChange({ read_timeout: v })}
                    />
                    <div className="col-span-2">
                        <div className="flex items-center gap-2 flex-wrap">
                            <Checkbox
                                id="evict-idle"
                                isSelected={poolIdleForm.evictIdle}
                                onChange={handleEvictToggle}
                                aria-controls="pool-idle-timeout"
                                aria-label="Evict idle connections"
                            >
                                <span className="settings-label !mb-0">Evict idle connections after</span>
                            </Checkbox>
                            <Input
                                id="pool-idle-timeout"
                                type="number"
                                inputMode="numeric"
                                min={1}
                                placeholder="90"
                                value={poolIdleRaw}
                                onChange={(e) => handlePoolIdleChange(e.target.value)}
                                disabled={!poolIdleForm.evictIdle}
                                aria-describedby="evict-idle-help"
                                aria-invalid={poolIdleError !== null}
                                className="font-mono text-xs w-20"
                            />
                            <span className="text-xs text-muted-foreground">s</span>
                        </div>
                        <p
                            id="evict-idle-help"
                            className={`text-xs mt-1 ${poolIdleError ? "text-destructive" : "text-muted-foreground"}`}
                        >
                            {poolIdleError ??
                                "When off, idle keep-alive connections are kept until the OS closes them."}
                        </p>
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
