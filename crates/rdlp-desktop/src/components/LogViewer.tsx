// LogViewer: virtualized monospace log viewer.
// Uses TanStack Virtual for rows, ref-based ring buffer (max 5000 entries).
// Subscribes to download log events via a shared log buffer.

import {
    useRef,
    useState,
    useCallback,
    useEffect,
    useSyncExternalStore,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

// ─── Ring buffer ────────────────────────────────────────────────────────────

export interface LogEntry {
    id: number;
    timestamp: number;
    level: "info" | "warn" | "debug" | "error";
    jobId: string | null;
    message: string;
}

const MAX_ENTRIES = 5000;

let _entries: LogEntry[] = [];
let _nextId = 0;
const _listeners = new Set<() => void>();

function _notify() {
    for (const l of _listeners) l();
}

export function appendLog(
    level: LogEntry["level"],
    message: string,
    jobId: string | null = null,
): void {
    const entry: LogEntry = { id: _nextId++, timestamp: Date.now(), level, jobId, message };
    _entries = _entries.length >= MAX_ENTRIES
        ? [..._entries.slice(_entries.length - MAX_ENTRIES + 1), entry]
        : [..._entries, entry];
    _notify();
}

export function clearLogs(): void {
    _entries = [];
    _notify();
}

function subscribeToLogs(cb: () => void): () => void {
    _listeners.add(cb);
    return () => _listeners.delete(cb);
}

function getLogSnapshot(): LogEntry[] {
    return _entries;
}

// ─── Component ───────────────────────────────────────────────────────────────

type SeverityFilter = "info" | "warn" | "debug" | "error";

const LEVEL_COLORS: Record<SeverityFilter, string> = {
    info: "text-[#aaaaaa]",
    warn: "text-[#e8a838]",
    error: "text-[#e85858]",
    debug: "text-[#555555]",
};

const LEVEL_BG: Record<SeverityFilter, string> = {
    info: "bg-[#1a2a4a] text-[#4a9eff]",
    warn: "bg-[#2a1f0a] text-[#e8a838]",
    error: "bg-[#2a0a0a] text-[#e85858]",
    debug: "bg-[#141414] text-[#555555]",
};

function formatTime(ts: number): string {
    const d = new Date(ts);
    return (
        String(d.getHours()).padStart(2, "0") +
        ":" +
        String(d.getMinutes()).padStart(2, "0") +
        ":" +
        String(d.getSeconds()).padStart(2, "0") +
        "." +
        String(d.getMilliseconds()).padStart(3, "0")
    );
}

export function LogViewer() {
    const allEntries = useSyncExternalStore(subscribeToLogs, getLogSnapshot);
    const [activeFilters, setActiveFilters] = useState<Set<SeverityFilter>>(
        new Set(["info", "warn", "error", "debug"]),
    );
    const [autoScroll, setAutoScroll] = useState(true);
    const parentRef = useRef<HTMLDivElement>(null);

    const filtered = allEntries.filter((e) => activeFilters.has(e.level as SeverityFilter));

    const virtualizer = useVirtualizer({
        count: filtered.length,
        getScrollElement: () => parentRef.current,
        estimateSize: () => 18,
        overscan: 10,
    });

    // Auto-scroll to bottom when new entries arrive
    useEffect(() => {
        if (autoScroll && filtered.length > 0) {
            virtualizer.scrollToIndex(filtered.length - 1, { behavior: "auto" });
        }
    }, [filtered.length, autoScroll, virtualizer]);

    const handleScroll = useCallback(() => {
        const el = parentRef.current;
        if (!el) return;
        const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        setAutoScroll(atBottom);
    }, []);

    const toggleFilter = (level: SeverityFilter) => {
        setActiveFilters((prev) => {
            const next = new Set(prev);
            if (next.has(level)) {
                if (next.size > 1) next.delete(level); // keep at least one
            } else {
                next.add(level);
            }
            return next;
        });
    };

    const items = virtualizer.getVirtualItems();

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Toolbar */}
            <div className="flex items-center gap-2 px-2 py-1 border-b border-[#1a1a2e] shrink-0">
                <div className="flex items-center gap-1">
                    {(["info", "warn", "error", "debug"] as SeverityFilter[]).map((level) => (
                        <button
                            key={level}
                            onClick={() => toggleFilter(level)}
                            className={cn(
                                "px-1.5 py-0.5 rounded-[3px] text-[9px] font-medium uppercase tracking-wide transition-colors",
                                activeFilters.has(level)
                                    ? LEVEL_BG[level]
                                    : "text-[#333333] bg-transparent hover:text-[#555555]",
                            )}
                        >
                            {level}
                        </button>
                    ))}
                </div>

                <span className="text-[10px] text-[#333333] font-mono ml-1">
                    {filtered.length}/{allEntries.length}
                </span>

                <div className="flex items-center gap-1 ml-auto">
                    {!autoScroll && (
                        <button
                            onClick={() => {
                                setAutoScroll(true);
                                virtualizer.scrollToIndex(filtered.length - 1);
                            }}
                            className="px-1.5 py-0.5 rounded-[3px] text-[9px] bg-[#1a2a4a] text-[#4a9eff] hover:bg-[#1f3050] transition-colors"
                        >
                            ↓ Follow
                        </button>
                    )}
                    <button
                        onClick={clearLogs}
                        aria-label="Clear logs"
                        className="p-1 rounded-[3px] text-[#333333] hover:text-[#666666] hover:bg-[#1a1a2e] transition-colors"
                    >
                        <X className="w-3 h-3" />
                    </button>
                </div>
            </div>

            {/* Virtual list */}
            {filtered.length === 0 ? (
                <div className="flex items-center justify-center flex-1">
                    <p className="text-[10px] text-[#333333] font-mono">No log entries</p>
                </div>
            ) : (
                <div
                    ref={parentRef}
                    onScroll={handleScroll}
                    className="flex-1 overflow-y-auto"
                    style={{ contain: "strict" }}
                >
                    <div
                        style={{
                            height: `${virtualizer.getTotalSize()}px`,
                            width: "100%",
                            position: "relative",
                        }}
                    >
                        {items.map((vItem) => {
                            const entry = filtered[vItem.index];
                            if (!entry) return null;
                            return (
                                <div
                                    key={entry.id}
                                    data-index={vItem.index}
                                    ref={virtualizer.measureElement}
                                    style={{
                                        position: "absolute",
                                        top: 0,
                                        left: 0,
                                        width: "100%",
                                        transform: `translateY(${vItem.start}px)`,
                                    }}
                                    className="flex items-start gap-2 px-2 py-px hover:bg-[#0e0e1e]"
                                >
                                    <span className="text-[10px] font-mono text-[#333333] shrink-0 select-none">
                                        {formatTime(entry.timestamp)}
                                    </span>
                                    <span
                                        className={cn(
                                            "text-[9px] font-mono uppercase tracking-wide shrink-0 w-[28px]",
                                            LEVEL_COLORS[entry.level as SeverityFilter] ?? "text-[#aaaaaa]",
                                        )}
                                    >
                                        {entry.level.slice(0, 4)}
                                    </span>
                                    <span
                                        className={cn(
                                            "text-[11px] font-mono break-all leading-relaxed",
                                            LEVEL_COLORS[entry.level as SeverityFilter] ?? "text-[#aaaaaa]",
                                        )}
                                    >
                                        {entry.message}
                                    </span>
                                </div>
                            );
                        })}
                    </div>
                </div>
            )}
        </div>
    );
}
