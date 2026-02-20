// Compact command bar replacing the top portion of the old SearchBar.
// Site selector + search input with shortcut hint + icon-only submit.

import { useEffect, useRef } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { Search, ArrowRight, Loader2 } from "lucide-react";
import { searchParamsAtom, setSearchParam } from "../stores/searchParamsStore";
import { providersQueryOptions, searchQueryOptions } from "../api/search";
import { Button } from "@/components/ui/button";

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
    const { isFetching, refetch } = useQuery(searchQueryOptions(query, site, filters));

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

    const isDisabled = isFetching || query.trim() === "";

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault();
        if (!isDisabled) {
            void refetch();
        }
    };

    const handleSiteChange = (newSite: string) => {
        setSearchParam("site", newSite);
    };

    return (
        <form className="flex items-center gap-1.5" onSubmit={handleSubmit} ref={formRef}>
            <select
                className="h-9 px-2.5 pr-7 border border-border rounded-md bg-card text-foreground text-xs font-medium cursor-pointer appearance-none transition-colors focus:outline-none focus:border-primary focus:ring-2 focus:ring-primary/20 bg-[length:10px_10px] bg-[position:right_8px_center] bg-no-repeat bg-[url(&quot;data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10' viewBox='0 0 24 24' fill='none' stroke='%238b8d93' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E&quot;)]"
                value={site}
                onChange={(e) => handleSiteChange(e.target.value)}
                disabled={isFetching}
                aria-label="Search site"
            >
                {providers.length === 0 && (
                    <option value="">Loading...</option>
                )}
                {providers.map((p) => (
                    <option key={p.name} value={p.name}>
                        {p.display_name}
                    </option>
                ))}
            </select>

            <div className="flex-1 relative flex items-center group">
                <Search className="absolute left-2.5 h-3.5 w-3.5 text-muted-foreground pointer-events-none transition-colors group-focus-within:text-primary" />
                <input
                    ref={inputRef}
                    className="peer w-full py-[7px] pl-8 pr-[70px] border border-border rounded-md bg-card text-foreground text-[13px] transition-colors placeholder:text-muted-foreground focus:outline-none focus:border-primary focus:ring-2 focus:ring-primary/20"
                    type="text"
                    placeholder="Search videos..."
                    value={query}
                    onChange={(e) => setSearchParam("query", e.target.value)}
                    disabled={isFetching}
                />
                <kbd className="absolute right-2 px-1.5 py-px rounded-sm bg-white/[0.04] border border-border text-muted-foreground font-mono text-[10px] pointer-events-none peer-focus:opacity-0">Ctrl+K</kbd>
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
