// QueryClient singleton with desktop-tuned defaults.
//
// Tauri apps differ from browser SPAs:
//   - No tab switching -> refetchOnWindowFocus is useless
//   - Backend state doesn't change externally -> higher staleTime
//   - No frontend retry (#700): every command result is the engine's FINAL
//     verdict — rdlp-core already retries transient network failures
//     (`Config::retries`, `is_retryable`). A rejected `invoke` means the
//     engine gave up, so re-invoking re-runs the whole extraction or search
//     against the target site. Measured: `retry: 1` produced two `formats`
//     command entries ~600 ms apart on every failing Analyze, in dev and in a
//     release build alike. The user retries manually via the Retry button.

import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            retry: false,
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
