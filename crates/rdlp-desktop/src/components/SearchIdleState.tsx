// Rich idle state: recent searches + keyboard shortcuts + quick start chips.

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
        <div className="idle-state">
            <div className="idle-state-columns">
                <div className="idle-state-section">
                    <h3 className="idle-state-heading">Recent Searches</h3>
                    {recent.length === 0 ? (
                        <p className="idle-state-empty-text">No recent searches</p>
                    ) : (
                        <div className="idle-state-recents">
                            {recent.map((entry) => (
                                <button
                                    key={`${entry.site}-${entry.query}`}
                                    className="idle-state-recent"
                                    onClick={() =>
                                        onRestoreSearch(entry.query, entry.site, entry.filters)
                                    }
                                >
                                    <svg className="idle-state-recent-icon" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                        <circle cx="11" cy="11" r="8" />
                                        <path d="M21 21l-4.3-4.3" />
                                    </svg>
                                    <span className="idle-state-recent-query">
                                        &ldquo;{entry.query}&rdquo;
                                    </span>
                                    <span className="idle-state-recent-meta">
                                        {entry.siteDisplayName} &middot;{" "}
                                        {formatDistanceToNow(entry.timestamp, { addSuffix: true })}
                                    </span>
                                </button>
                            ))}
                        </div>
                    )}
                </div>

                <div className="idle-state-section">
                    <h3 className="idle-state-heading">Keyboard Shortcuts</h3>
                    <div className="idle-state-shortcuts">
                        {SHORTCUTS.map((s) => (
                            <div key={s.keys} className="idle-state-shortcut">
                                <kbd className="idle-state-kbd">{s.keys}</kbd>
                                <span>{s.desc}</span>
                            </div>
                        ))}
                    </div>
                </div>
            </div>

            <div className="idle-state-quickstart">
                <span className="idle-state-quickstart-label">Try:</span>
                {QUICK_QUERIES.map((q) => (
                    <button
                        key={q}
                        className="idle-state-chip"
                        onClick={() => onQuickSearch(q)}
                    >
                        {q}
                    </button>
                ))}
            </div>
        </div>
    );
}
