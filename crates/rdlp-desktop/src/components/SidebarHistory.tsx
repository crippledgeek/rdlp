import { useState } from "react";
import { useSearchHistory } from "../hooks/useSearchHistory";

interface SidebarHistoryProps {
    onRestoreSearch: (query: string, site: string, filters: Array<{ key: string; value: string }>) => void;
}

export function SidebarHistory({ onRestoreSearch }: SidebarHistoryProps) {
    const { grouped, removeEntry, clearAll } = useSearchHistory();
    const [collapsed, setCollapsed] = useState(false);

    return (
        <div className="sidebar-section">
            <button
                className="sidebar-section-header"
                onClick={() => setCollapsed(!collapsed)}
                aria-expanded={!collapsed}
            >
                <svg
                    className={`sidebar-chevron ${collapsed ? "collapsed" : ""}`}
                    viewBox="0 0 16 16" width="10" height="10"
                    fill="currentColor"
                >
                    <path d="M4 6l4 4 4-4" />
                </svg>
                <span>HISTORY</span>
            </button>

            {!collapsed && (
                <div className="sidebar-section-content">
                    {grouped.length === 0 ? (
                        <div className="sidebar-empty">No recent searches</div>
                    ) : (
                        <>
                            {grouped.map((group) => (
                                <div key={group.site} className="sidebar-history-group">
                                    <div className="sidebar-history-site">
                                        {group.displayName}
                                    </div>
                                    {group.entries.map((entry) => (
                                        <div
                                            key={`${entry.site}-${entry.query}`}
                                            className="sidebar-history-item"
                                        >
                                            <button
                                                className="sidebar-history-query"
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
                                                className="sidebar-history-remove"
                                                onClick={() =>
                                                    removeEntry(entry.query, entry.site)
                                                }
                                                aria-label={`Remove "${entry.query}"`}
                                                title="Remove"
                                            >
                                                &times;
                                            </button>
                                        </div>
                                    ))}
                                </div>
                            ))}
                            <button
                                className="sidebar-clear-all"
                                onClick={clearAll}
                            >
                                Clear All
                            </button>
                        </>
                    )}
                </div>
            )}
        </div>
    );
}
