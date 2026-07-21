// NumericField: shared bounded-numeric settings control.
//
// Wraps React Aria's NumberField, which implements arrow-key increment/decrement
// and Home/End stepping over minValue/maxValue. Note: React Aria's
// useNumberField deliberately overrides the ARIA APG spinbutton role to `null`
// on the rendered <input> (VoiceOver focus incompatibility) — the input exposes
// the implicit `textbox` role plus `aria-roledescription="Number field"`
// instead, verified against the installed react-aria source. RAC's
// useNumberFieldState already owns the in-progress-text vs committed-number split
// and defers onChange to blur, so this component deliberately keeps NO local
// draft/error state — the hand-rolled version it replaces did, redundantly.
//
// Unit-agnostic by design: callers displaying a converted unit (e.g. MiB over a
// byte-valued setting) convert at the value/onCommit boundary and pass `suffix`.

import { Text } from "react-aria-components";
import { NumberField, NumberFieldInput, NumberFieldSteppers } from "@/components/ui/numberfield";
import { FieldError, FieldGroup, Label } from "@/components/ui/field";

interface NumericFieldProps {
    id: string;
    label: string;
    helper: string;
    /** Value in the DISPLAY unit; `null` renders an empty field ("inherit default"). */
    value: number | null;
    /**
     * Bounds are enforced by CLAMPING, not rejection: React Aria's
     * `useNumberFieldState.commit()` unconditionally clamps the committed
     * value to `[minValue, maxValue]` *before* any validation runs
     * (verified in `@react-stately/numberfield`'s `useNumberFieldState.mjs`).
     * An out-of-range entry is silently coerced to the nearest bound; it is
     * never rejected or surfaced as a `FieldError`. Do not add a schema
     * expecting rejection here — it would be unreachable dead code (see
     * `NumericField.test.tsx`'s clamping test).
     */
    minValue: number;
    maxValue: number;
    onCommit: (next: number | null) => void;
    suffix?: string;
    isDisabled?: boolean;
}

export function NumericField({
    id,
    label,
    helper,
    value,
    minValue,
    maxValue,
    onCommit,
    suffix,
    isDisabled = false,
}: NumericFieldProps) {
    return (
        <NumberField
            id={id}
            // RAC's `value` is `number | undefined` (ValueBase<number>); `null` is a
            // type error. With `exactOptionalPropertyTypes` on, an explicit
            // `value={undefined}` is ALSO a type error (the key must be omitted, not
            // present-but-undefined) — so the prop is spread in conditionally to
            // reach RAC's uncontrolled/empty sentinel.
            {...(value !== null ? { value } : {})}
            minValue={minValue}
            maxValue={maxValue}
            onChange={(n) => onCommit(Number.isNaN(n) ? null : n)}
            // "aria" surfaces validation inline without requiring a form submit.
            validationBehavior="aria"
            isDisabled={isDisabled}
            className="group flex flex-col gap-1"
        >
            {/* No aria-label: it would override this visible Label as the accessible
                name, and a drift between the two breaks WCAG 2.5.3 Label in Name. */}
            <Label className="settings-label">{label}</Label>
            <div className="flex items-center gap-1">
                <FieldGroup className="relative flex-1">
                    <NumberFieldInput className="font-mono text-xs" />
                    <NumberFieldSteppers />
                </FieldGroup>
                {suffix && <span className="text-xs text-muted-foreground">{suffix}</span>}
            </div>
            <Text className="text-xs text-muted-foreground" slot="description">
                {helper}
            </Text>
            <FieldError className="text-xs text-destructive" />
        </NumberField>
    );
}
