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

/**
 * `minValue`/`maxValue` bounds are enforced by CLAMPING, not rejection: React
 * Aria's `useNumberFieldState.commit()` unconditionally clamps the committed
 * value to `[minValue, maxValue]` *before* any validation runs (verified in
 * `@react-stately/numberfield`'s `useNumberFieldState.mjs`). An out-of-range
 * entry is silently coerced to the nearest bound; it is never rejected or
 * surfaced as a `FieldError`. Do not add a schema expecting rejection here —
 * it would be unreachable dead code (see `NumericField.test.tsx`'s clamping
 * test).
 */
interface NumericFieldProps {
    id: string;
    label: string;
    helper: string;
    /** Value in the DISPLAY unit; `null` renders an empty field ("inherit default"). */
    value: number | null;
    /** Lower bound; see the clamping note below for how `minValue`/`maxValue` are enforced. */
    minValue: number;
    /** Upper bound; see the clamping note below for how `minValue`/`maxValue` are enforced. */
    maxValue: number;
    onCommit: (next: number | null) => void;
    /** Shown in the empty input to hint the backend default ("inherit" affordance). */
    placeholder?: string;
    suffix?: string;
    isDisabled?: boolean;
    /**
     * Visually hides the `<Label>` (via `sr-only`) while keeping it in the DOM as the
     * field's accessible name. Use when a sibling control (e.g. a checkbox) already
     * carries the equivalent visible text and a second visible label would just repeat
     * it in a cramped layout. Never replace the `<Label>` with `aria-label` instead —
     * see the no-aria-label note below.
     */
    hideLabel?: boolean;
    /**
     * ID of an external element that describes this field, for callers that render
     * the helper text as a sibling (e.g. a full-width `FormDescription` outside a
     * narrow column) instead of passing `helper`. React Aria's `useField` merges this
     * with the field's own generated description/error ids into one `aria-describedby`
     * (verified in `@react-aria/label`'s `useField.mjs`), so the accessible description
     * stays wired even when `helper=""`.
     */
    "aria-describedby"?: string;
}

export function NumericField({
    id,
    label,
    helper,
    value,
    minValue,
    maxValue,
    onCommit,
    placeholder,
    suffix,
    isDisabled = false,
    hideLabel = false,
    "aria-describedby": ariaDescribedBy,
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
            // Every setting behind this component is integer-valued (seconds, counts,
            // whole MiB). Without `formatOptions`, React Aria's default `Intl.NumberFormat`
            // allows up to 3 fractional digits, so the decimal separator is accepted as
            // valid partial input and `step` has nothing to snap (verified against
            // `useNumberFieldState.mjs`: no `step` prop means no `snapValueToStep` call).
            // `maximumFractionDigits: 0` excludes the decimal separator from valid
            // *typed* input: React Aria's NumberParser rejects "." as an invalid
            // partial character when `maximumFractionDigits: 0` (verified in
            // `@internationalized/number`'s `NumberParser.mjs`), so typing "3.5"
            // never produces the value 3.5 — the "." keystroke is dropped and the
            // digits concatenate, leaving "35". Rounding to an integer only applies
            // to a fractional value supplied *programmatically* (e.g. a controlled
            // `value` prop), not to typed input.
            formatOptions={{ maximumFractionDigits: 0 }}
            {...(ariaDescribedBy !== undefined ? { "aria-describedby": ariaDescribedBy } : {})}
            onChange={(n) => onCommit(Number.isNaN(n) ? null : n)}
            // "aria" selects ARIA-based validation reporting over React Aria's
            // native browser constraint-validation UI. Nothing in this component
            // currently triggers validation (no `validate`/`isRequired`), so this
            // has no observable effect today; it only matters if React Aria's
            // built-in validation is ever turned on.
            validationBehavior="aria"
            isDisabled={isDisabled}
            className="group flex flex-col gap-1"
        >
            {/* No aria-label: it would override this visible Label as the accessible
                name, and a drift between the two breaks WCAG 2.5.3 Label in Name.
                `hideLabel` visually hides it via `sr-only` instead — it stays in the
                DOM as the accessible name. */}
            <Label className={hideLabel ? "sr-only" : "settings-label"}>{label}</Label>
            <div className="flex items-center gap-1">
                <FieldGroup className="relative flex-1">
                    <NumberFieldInput
                        className="font-mono text-xs"
                        {...(placeholder !== undefined ? { placeholder } : {})}
                    />
                    <NumberFieldSteppers />
                </FieldGroup>
                {suffix && <span className="text-xs text-muted-foreground">{suffix}</span>}
            </div>
            {/* Omit the description slot entirely when there is no helper text: React
                Aria's useField always wires `aria-describedby` to a rendered
                slot="description" node regardless of its content, so an
                unconditional empty <Text> here would give the field an empty
                accessible description. Callers with no local helper (e.g. a
                narrow-column field whose helper lives in a sibling
                FormDescription) pass `helper=""` and `aria-describedby` instead. */}
            {helper !== "" && (
                <Text className="text-xs text-muted-foreground" slot="description">
                    {helper}
                </Text>
            )}
            <FieldError className="text-xs text-destructive" />
        </NumberField>
    );
}
