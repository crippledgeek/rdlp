// Seeds the search site selector from the persisted default search provider.
//
// `AppSettings.default_search_provider` was persisted and read nowhere, so the
// CommandBar selector always started on "All sites" (#589). Seeding happens
// where the settings reach the frontend — the `settings` query — rather than
// at module scope, because that query is the only point at which the persisted
// value exists on this side of the IPC boundary.
//
// Vocabulary: "Auto"/"All sites" is stored as `null` in
// `AppSettings.default_search_provider` and as `""` in `searchStore.site`.
// Neither is the `NONE_KEY`/`NONE_SENTINEL` constant itself — both of those are
// the literal string `"none"`, a React Aria Select key (it rejects an empty
// key), converted at the selection handlers in `GeneralSection.tsx` and
// `CommandBar.tsx`. `searchStore.site` therefore never holds `"none"`, and a
// null default seeds nothing.

import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { settingsQueryOptions } from "@/api/settings";
import { providersQueryOptions } from "@/api/search";
import { searchStore, setSearchParam } from "@/stores/searchStore";

/** Seed `searchStore.site` from the persisted default provider, once. */
export function useSeedSearchProvider(): void {
    const { data: settings } = useQuery(settingsQueryOptions());
    const { data: providers } = useQuery(providersQueryOptions());
    const seeded = useRef(false);

    useEffect(() => {
        // Both queries must have resolved before the seed is spent — bailing
        // on `seeded.current` while either is still in flight would burn the
        // one attempt and seed nothing.
        if (seeded.current || !settings || !providers) return;
        seeded.current = true;

        const provider = settings.default_search_provider;
        // Any site the user picked while the query was in flight outranks the
        // stored default — including "All sites", which is spelled `""` and so
        // is indistinguishable from the resting state without `siteTouched`.
        // This seeds an untouched selector; it never resets a chosen one.
        if (!provider || searchStore.state.siteTouched) return;

        // A settings file can name a provider this build does not offer (stale
        // config, hand-edit, removed extractor). Seeding it would leave the
        // CommandBar Select with no item matching its `selectedKey`: the
        // control renders as unselected while the store holds the value, and
        // every search then fails at the backend allow-list with nothing on
        // screen to explain it.
        if (!providers.some((p) => p.name === provider)) return;

        setSearchParam("site", provider);
    }, [settings, providers]);
}
