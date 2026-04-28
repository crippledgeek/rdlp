// HistoryNav: nav panel content for the History view.
// Search/filter input and group-by selector.

import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { Search } from "lucide-react";
import { historyFilterStore, setHistorySearch, setHistoryGroupBy } from "@/stores/historyFilterStore";
import { downloadsQueryOptions } from "@/api/downloads";
import { cn } from "@/lib/utils";

type GroupBy = "date" | "site" | "type";
const groupOptions: { key: GroupBy; label: string }[] = [
    { key: "date", label: "By Date" },
    { key: "site", label: "By Site" },
    { key: "type", label: "By Type" },
];

export function HistoryNav() {
    const { search, groupBy } = useStore(historyFilterStore, (s) => s);
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    const completedCount = jobs.filter((j) => j.status === "completed" || j.status === "failed" || j.status === "cancelled").length;

    return (
        <div className="flex flex-col h-full overflow-y-auto p-2 gap-3">
            {/* Search */}
            <section>
                <h2 className="section-heading px-2 mb-1">Search</h2>
                <div className="relative mx-1">
                    <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-[var(--text-muted)] pointer-events-none" />
                    <input
                        type="text"
                        value={search}
                        onChange={(e) => setHistorySearch(e.target.value)}
                        placeholder="Filter history…"
                        className="w-full pl-7 pr-2 py-1 rounded-[4px] text-[12px] bg-[var(--surface-elevated)] border border-[#2a2a3e] text-[#aaaaaa] placeholder:text-[var(--text-muted)] outline-none focus:border-[#4a9eff]/50 transition-colors"
                    />
                </div>
            </section>

            {/* Group by */}
            <section>
                <h2 className="section-heading px-2 mb-1">Group By</h2>
                <div className="flex flex-col gap-0.5">
                    {groupOptions.map(({ key, label }) => (
                        <button
                            key={key}
                            onClick={() => setHistoryGroupBy(key)}
                            className={cn(
                                "text-left px-3 py-1.5 rounded-[4px] text-[12px] transition-colors",
                                groupBy === key
                                    ? "bg-[#1a2a4a] text-[#4a9eff]"
                                    : "text-[#aaaaaa] hover:bg-[#141428] hover:text-[#eeeeee]",
                            )}
                        >
                            {label}
                        </button>
                    ))}
                </div>
            </section>

            {/* Stats */}
            <section>
                <h2 className="section-heading px-2 mb-1">Stats</h2>
                <div className="px-3 text-[11px]">
                    <div className="flex justify-between">
                        <span className="text-[var(--text-muted)]">Total</span>
                        <span className="text-[#aaaaaa] font-mono">{completedCount}</span>
                    </div>
                    <p className="text-[10px] text-[var(--text-muted)] mt-1">Session only — no persistence yet</p>
                </div>
            </section>
        </div>
    );
}
