// Higher-order component that mounts the TanStack devtools panel alongside a
// root component, so `App` carries no devtools code at all.
//
// Nothing here is guarded on the environment. `@tanstack/devtools-vite` is
// registered first in `vite.config.ts`, and on a production build
// (`removeDevtoolsOnBuild` defaults to true) it strips the devtools imports and
// the `<TanStackDevtools>` JSX below. An `import.meta.env.DEV` check would add a
// branch that can rot without adding a guarantee — the strip is unconditional,
// a conditional is not. The docs recommend the hand-rolled conditional only for
// non-Vite projects.
//
// Be precise about what "strips" means, because the obvious reading is wrong:
// the transform REWRITES a file, it never removes one. It drops import
// declarations naming a package in `TANSTACK_DEVTOOLS_PACKAGES`, then the JSX
// elements they provided, then any panel import left unreferenced. So this
// module still ships — as `withDevtools` returning `<Wrapped {...props} />` and
// nothing else, a transparent wrapper. That residue is the 219 bytes the commit
// message measured; the measurement and this comment used to contradict each
// other, and the measurement was right.

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
    //
    // `||`, not `??`: a component can arrive with `name === ""` (an inline or
    // re-exported one), and `??` falls through only on null/undefined — so it
    // produced `withDevtools()` and the "Component" fallback was unreachable.
    // Caught by the test for that branch, which failed against `??`.
    const wrappedName = Wrapped.displayName || Wrapped.name || "Component";
    WithDevtools.displayName = `withDevtools(${wrappedName})`;

    return WithDevtools;
}
