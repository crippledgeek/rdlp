// QueryClient singleton with desktop-tuned defaults.
//
// Tauri apps differ from browser SPAs:
//   - No tab switching -> refetchOnWindowFocus is useless
//   - Backend state doesn't change externally -> higher staleTime
//   - User retries manually -> low retry count

import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            retry: 1,
            retryDelay: 500,
            staleTime: 30_000,
            gcTime: 5 * 60_000,
            refetchOnWindowFocus: false,
            refetchOnReconnect: true,
        },
        mutations: {
            retry: 0,
        },
    },
});

// Revoke Blob URLs when thumbnail proxy queries are garbage collected.
// Blob URLs are NOT revoked on component unmount (they must stay valid
// for TanStack Query cache hits on remount, e.g. after table sort).
// Instead, we clean them up here when the query is actually removed
// from the cache after gcTime expires.
queryClient.getQueryCache().subscribe((event) => {
    if (
        event.type === "removed" &&
        event.query.queryKey[0] === "proxy-thumbnail" &&
        typeof event.query.state.data === "string" &&
        event.query.state.data.startsWith("blob:")
    ) {
        URL.revokeObjectURL(event.query.state.data);
    }
});
