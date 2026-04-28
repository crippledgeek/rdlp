// IconRail: fixed 48px left column with 4 navigation icons.
// Active icon has 2px blue left edge indicator + tinted background.

import { Search, ArrowDownToLine, Clock, Settings } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { uiStore, setView } from "@/stores/uiStore";
import { downloadsQueryOptions } from "@/api/downloads";
import { cn } from "@/lib/utils";
import type { AppView } from "@/types";
import { TooltipTrigger } from "react-aria-components";
import { Button } from "@/components/ui/button";
import { Tooltip } from "@/components/ui/tooltip";

const NAV_ITEMS: { id: AppView; icon: LucideIcon; label: string }[] = [
    { id: "analyze", icon: Search, label: "Analyze (Ctrl+1)" },
    { id: "queue", icon: ArrowDownToLine, label: "Queue (Ctrl+2)" },
    { id: "history", icon: Clock, label: "History (Ctrl+3)" },
    { id: "settings", icon: Settings, label: "Settings (Ctrl+4)" },
];

export function IconRail() {
    const activeView = useStore(uiStore, (s) => s.activeView);
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    const activeCount = jobs.filter(
        (j) => j.status === "running" || j.status === "pending",
    ).length;

    return (
        <div
            className="flex flex-col items-center w-12 shrink-0 pt-2 pb-2 bg-[var(--surface-deepest)] border-r border-[#1a1a2e]"
            role="navigation"
            aria-label="Main navigation"
        >
            {NAV_ITEMS.map(({ id, icon: Icon, label }) => {
                const isActive = activeView === id;
                return (
                    <TooltipTrigger key={id} delay={600}>
                        <Button
                            variant="ghost"
                            size="icon"
                            onPress={() => setView(id)}
                            aria-label={label}
                            {...(isActive && { "aria-current": "page" as const })}
                            className={cn(
                                "relative flex items-center justify-center w-10 h-10 rounded-[6px] my-0.5 transition-colors",
                                isActive
                                    ? "bg-[#1a2a4a] text-[#4a9eff]"
                                    : "text-[var(--text-muted)] hover:text-[#aaaaaa] hover:bg-[#0e0e1e]",
                            )}
                        >
                            {/* Left edge active indicator */}
                            {isActive && (
                                <span className="absolute left-0 top-2 bottom-2 w-[2px] rounded-r-full bg-[#4a9eff]" />
                            )}
                            <Icon className="w-4 h-4" />
                            {/* Badge on queue icon */}
                            {id === "queue" && activeCount > 0 && (
                                <span className="absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] px-[3px] rounded-full bg-[#4a9eff] text-white text-[9px] font-bold flex items-center justify-center">
                                    {activeCount > 9 ? "9+" : activeCount}
                                </span>
                            )}
                        </Button>
                        <Tooltip placement="right">{label.split(" (")[0]}</Tooltip>
                    </TooltipTrigger>
                );
            })}
        </div>
    );
}
