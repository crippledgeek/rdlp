/**
 * Lightweight test mock for @/components/ui/radio-group (Radix RadioGroup).
 *
 * Renders native-like radio buttons with the same ARIA roles tests query:
 * - role="radio" on each item with aria-checked
 *
 * Avoids Radix RadioGroup's DOM overhead (roving focus, ARIA state machines).
 */
import * as React from "react";

interface RadioCtx {
    value: string;
    onValueChange?: (v: string) => void;
    disabled?: boolean;
}

const Ctx = React.createContext<RadioCtx | null>(null);

function RadioGroup({
    value,
    onValueChange,
    disabled,
    className,
    children,
}: {
    value?: string;
    onValueChange?: (v: string) => void;
    disabled?: boolean;
    className?: string;
    children: React.ReactNode;
}) {
    return (
        <Ctx.Provider value={{ value: value ?? "", onValueChange, disabled }}>
            <div role="radiogroup" className={className}>{children}</div>
        </Ctx.Provider>
    );
}

function RadioGroupItem({
    value,
    className,
    ...rest
}: {
    value: string;
    className?: string;
    disabled?: boolean;
} & React.HTMLAttributes<HTMLButtonElement>) {
    const ctx = React.useContext(Ctx);
    const checked = ctx?.value === value;
    return (
        <button
            role="radio"
            aria-checked={checked}
            className={className}
            onClick={() => ctx?.onValueChange?.(value)}
            disabled={ctx?.disabled || rest.disabled}
            {...rest}
        />
    );
}

export { RadioGroup, RadioGroupItem };
