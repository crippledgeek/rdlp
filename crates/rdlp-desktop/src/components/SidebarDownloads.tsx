import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { downloadsQueryOptions } from "../api/downloads";
import { cn } from "@/lib/utils";
import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
} from "@/components/ui/collapsible";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ChevronDown, Check, X, Circle } from "lucide-react";

interface SidebarDownloadsProps {
    onSwitchToQueue: () => void;
}

export function SidebarDownloads({ onSwitchToQueue }: SidebarDownloadsProps) {
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());
    const [collapsed, setCollapsed] = useState(false);

    const sorted = [...jobs].sort((a, b) => {
        const order: Record<string, number> = {
            running: 0, pending: 1, failed: 2, completed: 3, cancelled: 4,
        };
        return (order[a.status] ?? 5) - (order[b.status] ?? 5);
    }).slice(0, 5);

    const activeCount = jobs.filter(
        (j) => j.status === "pending" || j.status === "running",
    ).length;

    return (
        <Collapsible
            open={!collapsed}
            onOpenChange={(open) => setCollapsed(!open)}
            className="px-1 py-1"
        >
            <CollapsibleTrigger asChild>
                <button
                    className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground hover:bg-white/[0.04] transition-colors"
                    aria-expanded={!collapsed}
                >
                    <ChevronDown
                        className={cn(
                            "h-2.5 w-2.5 shrink-0 transition-transform",
                            collapsed && "-rotate-90",
                        )}
                    />
                    <span>Downloads</span>
                    {activeCount > 0 && (
                        <Badge
                            variant="default"
                            className="ml-auto h-4 min-w-4 px-1 text-[9px] leading-none"
                        >
                            {activeCount}
                        </Badge>
                    )}
                </button>
            </CollapsibleTrigger>

            <CollapsibleContent>
                <ScrollArea className="max-h-[300px]">
                    {sorted.length === 0 ? (
                        <div className="py-1.5 text-center text-[11px] text-muted-foreground">
                            No downloads
                        </div>
                    ) : (
                        <div className="flex flex-col gap-0.5 py-0.5">
                            {sorted.map((job) => (
                                <button
                                    key={job.id}
                                    className="flex w-full flex-col gap-1 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-white/[0.04]"
                                    onClick={onSwitchToQueue}
                                    title={job.title ?? job.url}
                                >
                                    <div className="flex items-center gap-1.5">
                                        {job.status === "completed" && (
                                            <Check className="h-3 w-3 shrink-0 text-primary" />
                                        )}
                                        {job.status === "failed" && (
                                            <X className="h-3 w-3 shrink-0 text-destructive" />
                                        )}
                                        {(job.status === "running" || job.status === "pending") && (
                                            <Circle className="h-3 w-3 shrink-0 animate-pulse text-primary" />
                                        )}
                                        <span className="truncate text-[11px] text-foreground">
                                            {job.title ?? "Untitled"}
                                        </span>
                                    </div>

                                    {(job.status === "running" || job.status === "pending") && (
                                        <>
                                            <Progress
                                                value={(job.progress ?? 0) * 100}
                                                className="h-[3px]"
                                            />
                                            <div className="flex gap-1 font-mono text-[10px] text-muted-foreground">
                                                {job.speed && <span>{job.speed}</span>}
                                                {job.eta && <span>&middot; {job.eta}</span>}
                                            </div>
                                        </>
                                    )}
                                </button>
                            ))}
                        </div>
                    )}
                </ScrollArea>
            </CollapsibleContent>
        </Collapsible>
    );
}
