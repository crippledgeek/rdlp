// Compact command bar replacing the top portion of the old SearchBar.
// Site selector + search input with shortcut hint + icon-only submit.

import { useEffect, useRef } from "react";
import { useSearchStore } from "../lib/store";

interface CommandBarProps {
    /** Ref exposed so keyboard nav can focus the input externally. */
    inputRef: React.RefObject<HTMLInputElement>;
    activeTab: string;
}

/** Compact 36px command bar: site selector, search input, submit icon. */
export function CommandBar({ inputRef, activeTab }: CommandBarProps) {
    const query = useSearchStore((s) => s.query);
    const setQuery = useSearchStore((s) => s.setQuery);
    const site = useSearchStore((s) => s.site);
    const setSite = useSearchStore((s) => s.setSite);
    const providers = useSearchStore((s) => s.providers);
    const loadProviders = useSearchStore((s) => s.loadProviders);
    const search = useSearchStore((s) => s.search);
    const status = useSearchStore((s) => s.status);

    const formRef = useRef<HTMLFormElement>(null);

    useEffect(() => {
        void loadProviders();
    }, [loadProviders]);

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

    const isDisabled = status === "loading" || query.trim() === "";

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault();
        if (!isDisabled) {
            void search();
        }
    };

    const handleSiteChange = (newSite: string) => {
        setSite(newSite);
    };

    return (
        <form className="command-bar" onSubmit={handleSubmit} ref={formRef}>
            <select
                className="command-bar-site"
                value={site}
                onChange={(e) => handleSiteChange(e.target.value)}
                disabled={status === "loading"}
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
                    onChange={(e) => setQuery(e.target.value)}
                    disabled={status === "loading"}
                />
                <kbd className="command-bar-shortcut">Ctrl+K</kbd>
            </div>

            <button
                type="submit"
                className="command-bar-submit"
                disabled={isDisabled}
                aria-label="Search"
            >
                {status === "loading" ? (
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
