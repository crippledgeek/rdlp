// Higher-order component that mounts the TanStack devtools panel alongside a
// root component, so `App` carries no devtools code at all.
//
// Nothing here is guarded on the environment. `@tanstack/devtools-vite` is
// registered first in `vite.config.ts` and removes this module and its imports
// from a production build (`removeDevtoolsOnBuild` defaults to true), so an
// `import.meta.env.DEV` check would add a branch that can rot without adding a
// guarantee — the strip is unconditional, a conditional is not. The docs
// recommend the hand-rolled conditional only for non-Vite projects.

import type { ComponentType } from "react";
import { TanStackDevtools } from "@tanstack/react-devtools";
import { ReactQueryDevtoolsPanel } from "@tanstack/react-query-devtools";
import { queryClient } from "../query/queryClient";

/**
 * Wrap a root component so the devtools render beside it.
 *
 * The client is passed to the panel explicitly rather than read from context:
 * wrapping from outside puts the devtools above `App`, and therefore above the
 * `QueryClientProvider` that lives inside it. `queryClient` is the same module
 * singleton that provider is given, so both see one cache.
 *
 * A panel added later that needs React context rather than an explicit handle
 * would not work at this level — it would have to move inside `App`, which is
 * the tradeoff this shape makes in exchange for keeping `App` free of devtools.
 */
export function withDevtools<P extends object>(
    Wrapped: ComponentType<P>,
): ComponentType<P> {
    function WithDevtools(props: P) {
        return (
            <>
                <Wrapped {...props} />
                <TanStackDevtools
                    plugins={[
                        {
                            name: "TanStack Query",
                            render: <ReactQueryDevtoolsPanel client={queryClient} />,
                        },
                    ]}
                />
            </>
        );
    }

    // Named for the React devtools tree, where an anonymous wrapper would
    // otherwise appear between the root and the app.
    WithDevtools.displayName = `withDevtools(${Wrapped.displayName ?? Wrapped.name ?? "Component"})`;

    return WithDevtools;
}
