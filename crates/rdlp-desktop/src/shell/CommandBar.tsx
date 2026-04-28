// CommandBar: unified 44px top bar for URL analysis and search.
// URL pattern (starts with http, or contains .) → analyze action.
// Text → search action with site selector.

import { useRef, useEffect, useState } from "react";
import { Search, X } from "lucide-react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import {
    Select,
    SelectItem,
    SelectListBox,
    SelectPopover,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";

const NONE_SENTINEL = "none";
import { uiStore, setView, setAnalyzeUrl } from "@/stores/uiStore";
import { searchStore, setSearchParam } from "@/stores/searchStore";
import { providersQueryOptions } from "@/api/search";
import { cn } from "@/lib/utils";

function isUrl(value: string): boolean {
    const trimmed = value.trim();
    return (
        trimmed.startsWith("http://") ||
        trimmed.startsWith("https://") ||
        (trimmed.length > 4 && trimmed.includes(".") && !trimmed.includes(" "))
    );
}

export function CommandBar() {
    const inputRef = useRef<HTMLInputElement>(null);
    // Draft tracks what the user is actively typing. null = not editing, show committed value.
    const [draft, setDraft] = useState<string | null>(null);
    const activeView = useStore(uiStore, (s) => s.activeView);
    const analyzeUrl = useStore(uiStore, (s) => s.analyzeUrl);
    const searchQuery = useStore(searchStore, (s) => s.query);
    const searchSite = useStore(searchStore, (s) => s.site);
    const { data: providers = [] } = useQuery(providersQueryOptions());

    // Display value: draft while editing, otherwise the committed source of truth
    const displayValue = draft ?? analyzeUrl ?? searchQuery;
    const inSearchMode = displayValue.length > 0 && !isUrl(displayValue);

    // Focus on Ctrl+K custom event or rdlp-focus-search event
    useEffect(() => {
        function onFocusSearch() {
            inputRef.current?.focus();
            inputRef.current?.select();
        }
        window.addEventListener("rdlp-focus-search", onFocusSearch);
        return () => window.removeEventListener("rdlp-focus-search", onFocusSearch);
    }, []);

    // Auto-paste on Ctrl+V when command bar is not focused
    useEffect(() => {
        async function onPaste(e: ClipboardEvent) {
            const target = e.target as HTMLElement;
            if (target === inputRef.current) return;
            const text = e.clipboardData?.getData("text/plain") ?? "";
            if (isUrl(text)) {
                handleAnalyzeUrl(text);
            }
        }
        document.addEventListener("paste", onPaste);
        return () => document.removeEventListener("paste", onPaste);
    }, []);

    function handleAnalyzeUrl(url: string) {
        const trimmed = url.trim();
        if (!trimmed) return;
        setView("analyze");
        setAnalyzeUrl(trimmed);
    }

    function handleSearch(query: string) {
        if (!query.trim()) return;
        setSearchParam("query", query.trim());
        if (activeView !== "analyze") setView("analyze");
    }

    function handleSubmit(e: React.FormEvent) {
        e.preventDefault();
        if (!displayValue.trim()) return;
        if (isUrl(displayValue)) {
            handleAnalyzeUrl(displayValue);
        } else {
            handleSearch(displayValue);
        }
        setDraft(null); // Commit: clear draft, show committed value
    }

    function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
        if (e.key === "Escape") {
            setDraft(null); // Discard draft, revert to committed value
            inputRef.current?.blur();
        }
    }

    function clearInput() {
        setDraft(null);
        setAnalyzeUrl(null);
        setSearchParam("query", "");
        inputRef.current?.focus();
    }

    return (
        <div
            className="flex items-center h-11 px-3 gap-2 border-b border-[#1a1a2e] bg-[var(--surface-raised)] shrink-0"
            style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        >
            <form
                onSubmit={handleSubmit}
                className="flex items-center flex-1 gap-2"
                style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
            >
                <Search className="w-3.5 h-3.5 text-[var(--text-muted)] shrink-0" />

                {/* URL/search input */}
                <input
                    ref={inputRef}
                    type="text"
                    value={displayValue}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Paste a URL or search…"
                    aria-label="URL or search query"
                    className={cn(
                        "flex-1 min-w-0 bg-transparent text-[13px] text-[#eeeeee] placeholder:text-[var(--text-muted)] outline-none",
                        "border-0 focus:outline-none",
                    )}
                    style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
                />

                {/* Site selector — hidden when not in search mode (CSS, not mount/unmount) */}
                {providers.length > 0 && (
                    <div style={{ display: inSearchMode ? undefined : "none" }}>
                        <Select
                            selectedKey={searchSite || NONE_SENTINEL}
                            onSelectionChange={(key) => setSearchParam("site", key === NONE_SENTINEL ? "" : String(key))}
                            aria-label="Search site"
                        >
                            <SelectTrigger
                                className="bg-[var(--surface-elevated)] border border-[#2a2a3e] rounded-[4px] text-[11px] text-[#aaaaaa] px-2 py-0.5 h-auto min-h-0 cursor-pointer"
                                style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
                            >
                                <SelectValue />
                            </SelectTrigger>
                            <SelectPopover>
                                <SelectListBox>
                                    <SelectItem id={NONE_SENTINEL} textValue="All sites">All sites</SelectItem>
                                    {providers.map((p) => (
                                        <SelectItem key={p.name} id={p.name} textValue={p.display_name}>
                                            {p.display_name}
                                        </SelectItem>
                                    ))}
                                </SelectListBox>
                            </SelectPopover>
                        </Select>
                    </div>
                )}

                {/* Clear button */}
                {displayValue && (
                    <Button
                        variant="ghost"
                        size="icon"
                        type="button"
                        onPress={clearInput}
                        aria-label="Clear input"
                        className="text-[var(--text-muted)] hover:text-[#aaaaaa] transition-colors h-auto bg-transparent p-0"
                        style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
                    >
                        <X className="w-3.5 h-3.5" />
                    </Button>
                )}

                {/* Action hint */}
                {!displayValue && (
                    <span className="kbd-chip shrink-0">Ctrl+K</span>
                )}
            </form>
        </div>
    );
}
