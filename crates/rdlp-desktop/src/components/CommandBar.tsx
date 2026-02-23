// Compact command bar replacing the top portion of the old SearchBar.
// Site selector + search input with shortcut hint + icon-only submit.

import { useEffect, useRef } from "react";
import { useStore } from "@tanstack/react-store";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Search, ArrowRight, Loader2, X } from "lucide-react";
import { searchParamsAtom, setSearchParam, resetSearchParams } from "../stores/searchParamsStore";
import { providersQueryOptions, searchInfiniteQueryOptions } from "../api/search";
import { Button } from "@/components/ui/button";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";

interface CommandBarProps {
    /** Ref exposed so keyboard nav can focus the input externally. */
    inputRef: React.RefObject<HTMLInputElement>;
    activeTab: string;
}

/** Compact 36px command bar: site selector, search input, submit icon. */
export function CommandBar({ inputRef, activeTab }: CommandBarProps) {
    const query = useStore(searchParamsAtom, (s) => s.query);
    const site = useStore(searchParamsAtom, (s) => s.site);
    const filters = useStore(searchParamsAtom, (s) => s.filters);

    const { data: providers = [] } = useQuery(providersQueryOptions());
    const { isFetching, refetch } = useInfiniteQuery(searchInfiniteQueryOptions(query, site, filters));

    const formRef = useRef<HTMLFormElement>(null);

    // Auto-select first provider when providers load and no site is set
    useEffect(() => {
        if (site === "" && providers.length > 0) {
            setSearchParam("site", providers[0].name);
        }
    }, [providers, site]);

    // Auto-focus when switching to search tab
    useEffect(() => {
        if (activeTab === "search") {
            inputRef.current?.focus();
        }
    }, [activeTab, inputRef]);

    // Listen for Ctrl+K custom event to focus the input
    useEffect(() => {
        const handler = () => inputRef.current?.focus();
        window.addEventListener("rdlp-focus-search", handler);
        return () => window.removeEventListener("rdlp-focus-search", handler);
    }, [inputRef]);

    const clearSearch = () => {
        const currentSite = site;
        resetSearchParams();
        setSearchParam("site", currentSite);
        inputRef.current?.focus();
    };

    const isDisabled = isFetching || query.trim() === "";

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault();
        if (!isDisabled) {
            refetch().catch((e) => console.error("Refetch failed:", e));
        }
    };

    const handleSiteChange = (newSite: string) => {
        setSearchParam("site", newSite);
    };

    return (
        <form className="flex items-center gap-1.5" onSubmit={handleSubmit} ref={formRef}>
            <Select
                value={site}
                onValueChange={handleSiteChange}
                disabled={isFetching}
            >
                <SelectTrigger
                    className="h-9 w-auto rounded-md border border-input bg-card px-2.5 text-xs font-medium text-foreground cursor-pointer transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                    aria-label="Search site"
                >
                    <SelectValue placeholder="Loading..." />
                </SelectTrigger>
                <SelectContent>
                    {providers.map((p) => (
                        <SelectItem key={p.name} value={p.name}>
                            {p.display_name}
                        </SelectItem>
                    ))}
                </SelectContent>
            </Select>

            <div className="flex-1 relative flex items-center group">
                <Search className="absolute left-2.5 h-3.5 w-3.5 text-muted-foreground pointer-events-none transition-colors group-focus-within:text-primary" />
                <input
                    ref={inputRef}
                    className="peer w-full py-[7px] pl-8 pr-[70px] border border-input rounded-md bg-card text-foreground text-[13px] transition-colors placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                    type="text"
                    placeholder="Search videos..."
                    value={query}
                    onChange={(e) => setSearchParam("query", e.target.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Escape" && query.length > 0) {
                            e.preventDefault();
                            clearSearch();
                        }
                    }}
                    disabled={isFetching}
                />
                {query.length > 0 && !isFetching && (
                    <button
                        type="button"
                        className="absolute right-14 p-0.5 rounded-sm text-muted-foreground hover:text-foreground transition-colors"
                        onClick={clearSearch}
                        aria-label="Clear search"
                    >
                        <X className="h-3.5 w-3.5" />
                    </button>
                )}
                <kbd className="absolute right-2 px-1.5 py-0.5 rounded-sm bg-white/[0.04] border border-white/[0.06] text-muted-foreground font-mono text-[10px] pointer-events-none peer-focus:opacity-0">Ctrl+K</kbd>
            </div>

            <Button
                type="submit"
                disabled={isDisabled}
                aria-label="Search"
                className="h-9 w-9 shrink-0 p-0 bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-35"
            >
                {isFetching ? (
                    <Loader2 className="size-3.5 animate-spin" />
                ) : (
                    <ArrowRight className="size-3.5" />
                )}
            </Button>
        </form>
    );
}
