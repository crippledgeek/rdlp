// Rich idle state: recent searches + keyboard shortcuts + quick start chips.

import { Search } from "lucide-react";
import { useSearchHistory } from "../hooks/useSearchHistory";
import { formatDistanceToNow } from "date-fns";

interface SearchIdleStateProps {
    currentSite: string;
    onRestoreSearch: (query: string, site: string, filters: Array<{ key: string; value: string }>) => void;
    onQuickSearch: (query: string) => void;
}

const SHORTCUTS = [
    { keys: "Ctrl+K", desc: "Focus search" },
    { keys: "Enter", desc: "Search / Download" },
    { keys: "\u2191 \u2193", desc: "Navigate results" },
    { keys: "Space", desc: "Select row" },
    { keys: "D", desc: "Quick download" },
    { keys: "F / Enter", desc: "Format dialog" },
    { keys: "Ctrl+B", desc: "Toggle sidebar" },
    { keys: "Ctrl+L", desc: "Toggle view" },
];

const QUICK_QUERIES = ["top rated", "trending", "new", "hd", "popular"];

/** Two-column idle state with recent searches, shortcuts, and quick start. */
export function SearchIdleState({
    currentSite: _currentSite,
    onRestoreSearch,
    onQuickSearch,
}: SearchIdleStateProps) {
    const { entries } = useSearchHistory();
    const recent = entries.slice(0, 5);

    return (
        <div className="p-6">
            <div className="grid grid-cols-[1.2fr_1fr] gap-6 mb-6">
                <div>
                    <h3 className="text-[10px] font-semibold text-muted-foreground tracking-wider uppercase mb-2.5">
                        Recent Searches
                    </h3>
                    {recent.length === 0 ? (
                        <p className="text-muted-foreground text-xs">No recent searches</p>
                    ) : (
                        <div className="flex flex-col gap-0.5">
                            {recent.map((entry) => (
                                <button
                                    key={`${entry.site}-${entry.query}`}
                                    className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-sm bg-transparent border-none cursor-pointer text-left w-full transition-colors text-foreground hover:bg-card"
                                    onClick={() =>
                                        onRestoreSearch(entry.query, entry.site, entry.filters)
                                    }
                                >
                                    <Search className="h-3 w-3 shrink-0 text-muted-foreground" />
                                    <span className="text-[13px] font-medium truncate">
                                        &ldquo;{entry.query}&rdquo;
                                    </span>
                                    <span className="text-[11px] text-muted-foreground whitespace-nowrap ml-auto">
                                        {entry.siteDisplayName} &middot;{" "}
                                        {formatDistanceToNow(entry.timestamp, { addSuffix: true })}
                                    </span>
                                </button>
                            ))}
                        </div>
                    )}
                </div>

                <div>
                    <h3 className="text-[10px] font-semibold text-muted-foreground tracking-wider uppercase mb-2.5">
                        Keyboard Shortcuts
                    </h3>
                    <div className="flex flex-col gap-1">
                        {SHORTCUTS.map((s) => (
                            <div key={s.keys} className="flex items-center gap-2.5 text-xs text-muted-foreground">
                                <kbd className="kbd-chip inline-block min-w-16 text-center">
                                    {s.keys}
                                </kbd>
                                <span>{s.desc}</span>
                            </div>
                        ))}
                    </div>
                </div>
            </div>

            <div className="flex items-center gap-1.5 flex-wrap">
                <span className="text-xs text-muted-foreground">Try:</span>
                {QUICK_QUERIES.map((q) => (
                    <button
                        key={q}
                        className="px-3 py-1 border border-white/[0.06] rounded-md bg-transparent text-muted-foreground text-xs cursor-pointer transition-colors hover:border-primary hover:text-primary hover:bg-primary/[0.12]"
                        onClick={() => onQuickSearch(q)}
                    >
                        {q}
                    </button>
                ))}
            </div>
        </div>
    );
}
