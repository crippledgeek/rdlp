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
