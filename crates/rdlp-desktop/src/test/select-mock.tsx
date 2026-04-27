/**
 * Lightweight test mock for @/components/ui/select.
 *
 * The real implementation is a React Aria (Jolly UI) Select — there are
 * no Radix interactive components in this project by policy. This mock
 * renders a native-like select with the same ARIA roles that tests query:
 * - role="combobox" on the trigger (with text content from the selected item)
 * - role="option" on each item (visible only when open)
 *
 * Avoids the heavy real DOM tree (portals, popover state, pointer capture,
 * full ARIA wiring) that costs ~40-80ms per render in tests.
 */
import * as React from "react";

// ---------------------------------------------------------------------------
// Context to wire parent <Select> state to children
// ---------------------------------------------------------------------------

interface SelectCtx {
    value: string;
    onValueChange?: ((v: string) => void) | undefined;
    open: boolean;
    setOpen: (o: boolean) => void;
    disabled?: boolean | undefined;
    items: Map<string, string>; // value → label text
}

const Ctx = React.createContext<SelectCtx | null>(null);
function useCtx() {
    const c = React.useContext(Ctx);
    if (!c) throw new Error("Select.* used outside <Select>");
    return c;
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

function Select({
    value,
    onValueChange,
    disabled,
    children,
}: {
    value?: string;
    onValueChange?: (v: string) => void;
    disabled?: boolean;
    children: React.ReactNode;
}) {
    const [open, setOpen] = React.useState(false);
    const [items] = React.useState(() => new Map<string, string>());
    return (
        <Ctx.Provider value={{ value: value ?? "", onValueChange, open, setOpen, disabled, items }}>
            <div data-slot="select">{children}</div>
        </Ctx.Provider>
    );
}

function SelectTrigger({
    children,
    className,
    size: _size,
    ...rest
}: {
    children: React.ReactNode;
    className?: string;
    size?: string;
} & React.HTMLAttributes<HTMLButtonElement>) {
    const ctx = useCtx();
    return (
        <button
            role="combobox"
            aria-expanded={ctx.open}
            disabled={ctx.disabled}
            className={className}
            onPointerDown={(e) => {
                if (ctx.disabled) return;
                e.preventDefault();
                ctx.setOpen(!ctx.open);
            }}
            {...rest}
        >
            {children}
        </button>
    );
}

function SelectValue({ placeholder }: { placeholder?: string }) {
    const ctx = useCtx();
    // Force a re-render after mount so we can read items registered by
    // SelectItem during the same commit (SelectItem renders after us in
    // the JSX tree, so items aren't available on first render).
    const [, forceUpdate] = React.useState(0);
    React.useEffect(() => { forceUpdate(1); }, []);
    const label = ctx.items.get(ctx.value);
    return <span data-slot="select-value">{label ?? placeholder ?? ""}</span>;
}

function SelectContent({ children }: { children: React.ReactNode }) {
    const ctx = useCtx();
    // Always render children so SelectItems can register their labels for
    // SelectValue display. Hide visually when closed; show as role="listbox"
    // when open so tests can query role="option".
    return (
        <div role="listbox" style={ctx.open ? undefined : { display: "none" }}>
            {children}
        </div>
    );
}

function SelectItem({
    value,
    children,
    ...rest
}: {
    value: string;
    children: React.ReactNode;
} & React.HTMLAttributes<HTMLDivElement>) {
    const ctx = useCtx();
    // Register label text synchronously so SelectValue can display it
    // in the same render pass (useEffect would be too late).
    const label = typeof children === "string" ? children : "";
    ctx.items.set(value, label);

    return (
        <div
            role="option"
            aria-selected={ctx.value === value}
            onClick={() => {
                ctx.onValueChange?.(value);
                ctx.setOpen(false);
            }}
            {...rest}
        >
            {children}
        </div>
    );
}

// Passthrough stubs for components tests don't query
function SelectGroup({ children }: { children: React.ReactNode }) {
    return <>{children}</>;
}
function SelectLabel({ children }: { children: React.ReactNode }) {
    return <span>{children}</span>;
}
function SelectSeparator() { return <hr />; }
function SelectScrollUpButton() { return null; }
function SelectScrollDownButton() { return null; }

export {
    Select,
    SelectContent,
    SelectGroup,
    SelectItem,
    SelectLabel,
    SelectScrollDownButton,
    SelectScrollUpButton,
    SelectSeparator,
    SelectTrigger,
    SelectValue,
};
