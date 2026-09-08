// Regression guard for #589: `default_search_provider` was persisted by the
// settings pane and read nowhere — the CommandBar site selector always started
// on "All sites". The hook seeds `searchStore.site` once, when the settings
// query first resolves.

import { describe, it, expect, afterEach } from "vitest";
import { useQuery } from "@tanstack/react-query";
import { render, screen, waitFor } from "@/test/test-utils";
import { clearInvokeHandlers, setInvokeHandler } from "@/test/tauri-mock";
import { settingsQueryOptions } from "@/api/settings";
import { providersQueryOptions } from "@/api/search";
import { searchStore, resetSearchParams, setSearchSite } from "@/stores/searchStore";
import { useSeedSearchProvider } from "./useSeedSearchProvider";
import type { AppSettings } from "@/types";

/** Minimal settings payload — only the field under test is load-bearing. */
function settingsWith(provider: string | null) {
    return { default_search_provider: provider } as unknown as AppSettings;
}

/** Register both queries the hook depends on. */
function mockBackend(provider: string | null, availableProviders: string[]) {
    setInvokeHandler("settings", () => settingsWith(provider));
    setInvokeHandler("search_providers", () =>
        availableProviders.map((name) => ({ name, display_name: name })),
    );
}

// The probe reports when BOTH queries the hook reads have resolved. The
// absence-asserting tests below need that signal: without it they assert "the
// site did not change" at a moment when the seed could not have run yet, and
// pass on timing alone — measured, they stayed green against a hook with the
// clobber guard deleted.
function Probe() {
    useSeedSearchProvider();
    const { data: settings } = useQuery(settingsQueryOptions());
    const { data: providers } = useQuery(providersQueryOptions());
    return <div>{settings && providers ? "queries-loaded" : "queries-pending"}</div>;
}

afterEach(() => {
    clearInvokeHandlers();
    resetSearchParams();
});

describe("useSeedSearchProvider", () => {
    it("seeds the search site from the configured default provider", async () => {
        mockBackend("xhamster", ["xhamster", "pornhub"]);

        render(<Probe />);

        await waitFor(() => expect(searchStore.state.site).toBe("xhamster"));
    });

    it("leaves the site on All sites when no provider is configured", async () => {
        mockBackend(null, ["xhamster", "pornhub"]);

        render(<Probe />);

        expect(await screen.findByText("queries-loaded")).toBeInTheDocument();
        expect(searchStore.state.site).toBe("");
    });

    it("does not clobber a site the user already picked", async () => {
        mockBackend("xhamster", ["xhamster", "pornhub"]);
        setSearchSite("pornhub");

        render(<Probe />);

        expect(await screen.findByText("queries-loaded")).toBeInTheDocument();
        expect(searchStore.state.site).toBe("pornhub");
    });

    // "All sites" is a deliberate choice that happens to be spelled `""` — the
    // same value as the untouched resting state. Gating the seed on
    // `site !== ""` therefore overwrites it, reproducing the exact
    // unspecified-vs-explicitly-empty collapse this commit fixes on the Rust
    // side. Only the `siteTouched` flag can tell the two apart.
    it("does not clobber a deliberate All sites pick", async () => {
        mockBackend("xhamster", ["xhamster", "pornhub"]);
        setSearchSite("");

        render(<Probe />);

        expect(await screen.findByText("queries-loaded")).toBeInTheDocument();
        expect(searchStore.state.site).toBe("");
    });

    // A stale or hand-edited settings file can name a provider this build does
    // not have. Seeding it would leave the Select with no matching item — the
    // control renders empty while the store holds the value, and every search
    // fails at the backend allow-list with nothing on screen explaining why.
    it("ignores a configured provider that is not in the allow-list", async () => {
        mockBackend("removedsite", ["xhamster", "pornhub"]);

        render(<Probe />);

        expect(await screen.findByText("queries-loaded")).toBeInTheDocument();
        expect(searchStore.state.site).toBe("");
    });
});
