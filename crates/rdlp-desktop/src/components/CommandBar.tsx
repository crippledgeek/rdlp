// Compact command bar replacing the top portion of the old SearchBar.
// Site selector + search input with shortcut hint + icon-only submit.

import { useEffect, useRef } from "react";
import { useStore } from "@tanstack/react-store";
import { useQuery } from "@tanstack/react-query";
import { searchParamsAtom, setSearchParam } from "../stores/searchParamsStore";
import { providersQueryOptions, searchQueryOptions } from "../api/search";

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
        <form className="command-bar" onSubmit={handleSubmit} ref={formRef}>
            <select
                className="command-bar-site"
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

            <div className="command-bar-input-wrapper">
                <svg
                    className="command-bar-search-icon"
                    viewBox="0 0 24 24" width="14" height="14"
                    fill="none" stroke="currentColor"
                    strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                >
                    <circle cx="11" cy="11" r="8" />
                    <path d="M21 21l-4.3-4.3" />
                </svg>
                <input
                    ref={inputRef}
                    className="command-bar-input"
                    type="text"
                    placeholder="Search videos..."
                    value={query}
                    onChange={(e) => setSearchParam("query", e.target.value)}
                    disabled={isFetching}
                />
                <kbd className="command-bar-shortcut">Ctrl+K</kbd>
            </div>

            <button
                type="submit"
                className="command-bar-submit"
                disabled={isDisabled}
                aria-label="Search"
            >
                {isFetching ? (
                    <span className="command-bar-spinner" />
                ) : (
                    <svg
                        viewBox="0 0 24 24" width="14" height="14"
                        fill="none" stroke="currentColor"
                        strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"
                    >
                        <line x1="5" y1="12" x2="19" y2="12" />
                        <polyline points="12 5 19 12 12 19" />
                    </svg>
                )}
            </button>
        </form>
    );
}
