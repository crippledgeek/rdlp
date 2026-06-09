// KnobRow: pure presentational component that renders one speed-control knob.
// A "choice" knob renders a Select; an "int" knob renders a bounded number input.
// Bound to caller-managed form state via value/onChange/onBlur.

import type { SpeedKnob } from "@/types";
import {
    Select,
    SelectItem,
    SelectListBox,
    SelectPopover,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";

const NONE_SENTINEL = "none";

/** One descriptor knob, bound to a string value + change/blur handlers by the caller. */
export function KnobRow({
    knob,
    value,
    onChange,
    onBlur,
}: {
    knob: SpeedKnob;
    value: string;
    onChange: (v: string) => void;
    onBlur: () => void;
}) {
    return (
        <div className="flex items-center justify-between pl-3">
            <span className="text-[10px] text-[var(--text-muted)]">{knob.label}</span>
            {knob.kind === "choice" ? (
                <Select
                    selectedKey={value || NONE_SENTINEL}
                    onSelectionChange={(key) =>
                        onChange(key === NONE_SENTINEL ? "" : String(key))
                    }
                    aria-label={knob.label}
                >
                    <SelectTrigger className="h-5 min-h-0 px-1.5 py-0 text-[10px] bg-[var(--surface-elevated)] border-[#2a2a3e] rounded-[3px] text-[var(--text-muted)] w-[90px]">
                        <SelectValue />
                    </SelectTrigger>
                    <SelectPopover>
                        <SelectListBox>
                            <SelectItem
                                id={NONE_SENTINEL}
                                textValue={`Default (${knob.default})`}
                            >
                                Default ({knob.default})
                            </SelectItem>
                            {knob.choices.map((c) => (
                                <SelectItem key={c} id={c} textValue={c}>
                                    {c}
                                </SelectItem>
                            ))}
                        </SelectListBox>
                    </SelectPopover>
                </Select>
            ) : (
                <Input
                    type="number"
                    min={knob.min}
                    max={knob.max}
                    value={value}
                    onBlur={onBlur}
                    onChange={(e) => onChange(e.target.value)}
                    placeholder={String(knob.default)}
                    className="h-5 w-[90px] px-1.5 py-0 rounded-[3px] bg-[var(--surface-elevated)] border border-[#2a2a3e] text-[10px] text-[var(--text-muted)] placeholder:text-[var(--text-muted)] outline-none focus:border-[#4a9eff]"
                />
            )}
        </div>
    );
}
