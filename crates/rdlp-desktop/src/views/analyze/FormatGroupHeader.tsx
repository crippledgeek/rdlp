// FormatGroupHeader: sticky section divider for Video+Audio / Video Only / Audio Only.

interface FormatGroupHeaderProps {
    label: string;
    count: number;
}

export function FormatGroupHeader({ label, count }: FormatGroupHeaderProps) {
    return (
        <tr className="sticky top-0 z-10">
            <td
                colSpan={7}
                className="py-1 px-3 bg-[var(--surface-deepest)] border-y border-[#1a1a2e]"
            >
                <span className="text-[10px] font-bold uppercase tracking-widest text-[var(--text-muted)]">
                    {label}
                </span>
                <span className="ml-2 text-[10px] text-[var(--text-muted)]">{count}</span>
            </td>
        </tr>
    );
}
