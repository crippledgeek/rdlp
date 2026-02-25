// Custom render wrapping QueryClientProvider with a fresh client per test.

import React from "react";
import { render, type RenderOptions } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";

/** Create a test QueryClient with retries disabled and data treated as fresh
 *  so pre-populated queries (via setQueryData) don't trigger background refetches. */
export function createTestQueryClient(): QueryClient {
    return new QueryClient({
        defaultOptions: {
            queries: {
                retry: false,
                gcTime: 0,
                staleTime: Infinity,
            },
            mutations: {
                retry: false,
            },
        },
    });
}

interface WrapperProps {
    children: React.ReactNode;
}

interface RenderWithProvidersOptions extends Omit<RenderOptions, "wrapper"> {
    /** Supply a pre-configured QueryClient (e.g. with setQueryData). */
    queryClient?: QueryClient;
}

/** Render with a fresh QueryClientProvider per test. */
export function renderWithProviders(
    ui: ReactElement,
    { queryClient: passedClient, ...renderOptions }: RenderWithProvidersOptions = {},
) {
    const queryClient = passedClient ?? createTestQueryClient();

    function Wrapper({ children }: WrapperProps) {
        return (
            <QueryClientProvider client={queryClient}>
                {children}
            </QueryClientProvider>
        );
    }

    return {
        queryClient,
        ...render(ui, { wrapper: Wrapper, ...renderOptions }),
    };
}

// Re-export everything from Testing Library for convenience.
export * from "@testing-library/react";
export { renderWithProviders as render };
