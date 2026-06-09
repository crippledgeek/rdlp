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
    // React Aria (Jolly UI) Select uses selectedKey/onSelectionChange instead of value/onValueChange.
    onSelectionChange?: ((key: string) => void) | undefined;
    open: boolean;
    setOpen: (o: boolean) => void;
    disabled?: boolean | undefined;
    items: Map<string, string>; // value/id → label text
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
    // React Aria (Jolly UI) props — used by KnobRow and other desktop components
    selectedKey,
    onSelectionChange,
    disabled,
    children,
}: {
    value?: string;
    onValueChange?: (v: string) => void;
    selectedKey?: string;
    onSelectionChange?: (key: string) => void;
    disabled?: boolean;
    children: React.ReactNode;
}) {
    const [open, setOpen] = React.useState(false);
    const [items] = React.useState(() => new Map<string, string>());
    // Support both shadcn (value/onValueChange) and React Aria (selectedKey/onSelectionChange) APIs.
    const resolvedValue = selectedKey ?? value ?? "";
    return (
        <Ctx.Provider value={{ value: resolvedValue, onValueChange, onSelectionChange, open, setOpen, disabled, items }}>
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
    // React Aria (Jolly UI) uses `id` as the item key, not `value`.
    id,
    children,
    textValue: _textValue,
    ...rest
}: {
    value?: string;
    id?: string;
    children: React.ReactNode;
    textValue?: string;
} & React.HTMLAttributes<HTMLDivElement>) {
    const ctx = useCtx();
    // The item's key is `id` (React Aria) or `value` (shadcn); fall back to the other.
    const key = id ?? value ?? "";
    // Register label text synchronously so SelectValue can display it
    // in the same render pass (useEffect would be too late).
    const label = typeof children === "string" ? children : "";
    ctx.items.set(key, label);

    return (
        <div
            role="option"
            aria-selected={ctx.value === key}
            onClick={() => {
                // Fire whichever callback the parent wired up.
                ctx.onSelectionChange?.(key);
                ctx.onValueChange?.(key);
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

// React Aria-style aliases used by the desktop's Jolly UI Select.
// SelectPopover wraps the listbox; SelectListBox is the listbox container.
// Map both to SelectContent so existing test queries (role="listbox", role="option") still work.
const SelectPopover = SelectContent;
const SelectListBox = ({ children }: { children: React.ReactNode }) => <>{children}</>;

export {
    Select,
    SelectContent,
    SelectGroup,
    SelectItem,
    SelectLabel,
    SelectListBox,
    SelectPopover,
    SelectScrollDownButton,
    SelectScrollUpButton,
    SelectSeparator,
    SelectTrigger,
    SelectValue,
};
