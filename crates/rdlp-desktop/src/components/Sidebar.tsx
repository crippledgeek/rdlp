import { SidebarDownloads } from "./SidebarDownloads";
import { SidebarHistory } from "./SidebarHistory";
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
        <aside className={`sidebar ${collapsed ? "collapsed" : ""}`}>
            <SidebarDownloads onSwitchToQueue={onSwitchToQueue} />
            <SidebarHistory onRestoreSearch={onRestoreSearch} />
        </aside>
    );
}
