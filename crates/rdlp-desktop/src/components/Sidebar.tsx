import { SidebarDownloads } from "./SidebarDownloads";
import { SidebarHistory } from "./SidebarHistory";
import { cn } from "@/lib/utils";
import type { SearchFilter } from "../types";

interface SidebarProps {
    collapsed: boolean;
    onSwitchToQueue: () => void;
    onRestoreSearch: (query: string, site: string, filters: SearchFilter[]) => void;
}

export function Sidebar({
    collapsed,
    onSwitchToQueue,
    onRestoreSearch,
}: SidebarProps) {
    return (
        <aside
            className={cn(
                "w-60 shrink-0 bg-background border-r border-border flex flex-col overflow-hidden transition-[width] duration-150 ease-out",
                collapsed && "w-0 border-r-0",
            )}
        >
            <SidebarDownloads onSwitchToQueue={onSwitchToQueue} />
            <SidebarHistory onRestoreSearch={onRestoreSearch} />
        </aside>
    );
}
