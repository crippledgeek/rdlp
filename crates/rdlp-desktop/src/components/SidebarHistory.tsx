import { useState } from "react";
import { useSearchHistory } from "../hooks/useSearchHistory";
import { cn } from "@/lib/utils";
import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
} from "@/components/ui/collapsible";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { ChevronDown, X } from "lucide-react";

interface SidebarHistoryProps {
    onRestoreSearch: (query: string, site: string, filters: Array<{ key: string; value: string }>) => void;
}

export function SidebarHistory({ onRestoreSearch }: SidebarHistoryProps) {
    const { grouped, removeEntry, clearAll } = useSearchHistory();
    const [collapsed, setCollapsed] = useState(false);

    return (
        <Collapsible
            open={!collapsed}
            onOpenChange={(open) => setCollapsed(!open)}
            className="px-1 py-1"
        >
            <CollapsibleTrigger asChild>
                <button
                    className="sidebar-section-btn"
                    aria-expanded={!collapsed}
                >
                    <ChevronDown
                        className={cn(
                            "h-2.5 w-2.5 shrink-0 transition-transform",
                            collapsed && "-rotate-90",
                        )}
                    />
                    <span>History</span>
                </button>
            </CollapsibleTrigger>

            <CollapsibleContent>
                <ScrollArea className="max-h-[300px]">
                    {grouped.length === 0 ? (
                        <div className="py-1.5 text-center text-[11px] text-muted-foreground">
                            No recent searches
                        </div>
                    ) : (
                        <div className="flex flex-col gap-0.5 py-0.5">
                            {grouped.map((group) => (
                                <div key={group.site}>
                                    <div className="px-2 py-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                                        {group.displayName}
                                    </div>
                                    {group.entries.map((entry) => (
                                        <div
                                            key={`${entry.site}-${entry.query}`}
                                            className="group flex items-center gap-0.5"
                                        >
                                            <button
                                                className="flex-1 truncate rounded-md px-2 py-1 text-left text-[11px] text-foreground transition-colors hover:bg-white/[0.04]"
                                                onClick={() =>
                                                    onRestoreSearch(
                                                        entry.query,
                                                        entry.site,
                                                        entry.filters,
                                                    )
                                                }
                                                title={`Search "${entry.query}" on ${group.displayName}`}
                                            >
                                                &middot; &ldquo;{entry.query}&rdquo;
                                            </button>
                                            <button
                                                className="shrink-0 rounded-md p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-white/[0.04] hover:text-foreground group-hover:opacity-100"
                                                onClick={() =>
                                                    removeEntry(entry.query, entry.site)
                                                }
                                                aria-label={`Remove "${entry.query}"`}
                                                title="Remove"
                                            >
                                                <X className="h-3 w-3" />
                                            </button>
                                        </div>
                                    ))}
                                </div>
                            ))}

                            <Separator className="my-1" />

                            <button
                                className="ml-auto rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-white/[0.04] hover:text-foreground"
                                onClick={clearAll}
                            >
                                Clear All
                            </button>
                        </div>
                    )}
                </ScrollArea>
            </CollapsibleContent>
        </Collapsible>
    );
}
