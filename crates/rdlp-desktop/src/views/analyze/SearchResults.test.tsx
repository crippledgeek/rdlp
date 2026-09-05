// Regression guard for the search error surface.
//
// The backend answers a Cloudflare-gated PornoXO search with an actionable
// message ("Pass --cookies-from-browser ... or use route=tag"). The error
// branch here used to destructure only `isError`, so that message was
// discarded and the operator saw a bare "Search failed" beside a Retry
// button that could never succeed.

import { describe, it, expect, afterEach } from "vitest";
import { render, screen } from "@/test/test-utils";
import { clearInvokeHandlers, setInvokeHandler } from "@/test/tauri-mock";
import { SearchResults } from "./SearchResults";

/** The wire shape Tauri produces for `AppError::SearchFailed`. */
function searchFailure(message: string) {
    return { kind: "SearchFailed", data: { message, retryable: true } };
}

const CLOUDFLARE_MESSAGE =
    "PornoXO search is behind a Cloudflare challenge. Pass \
--cookies-from-browser <browser> after solving it once in that browser, or \
use --search-filter route=tag to list a tag instead (which returns different \
results).";

afterEach(() => {
    clearInvokeHandlers();
});

describe("SearchResults error state", () => {
    it("renders the backend's actionable message, not just 'Search failed'", async () => {
        setInvokeHandler("search_content", () => {
            throw searchFailure(CLOUDFLARE_MESSAGE);
        });

        render(<SearchResults query="Lillyy" site="pornoxo" />);

        expect(
            await screen.findByText(/behind a Cloudflare challenge/i),
        ).toBeInTheDocument();
        // The advice is the load-bearing half: it names both escape routes.
        expect(screen.getByText(/--cookies-from-browser/)).toBeInTheDocument();
        expect(screen.getByText(/route=tag/)).toBeInTheDocument();
    });

    it("surfaces a plain-string rejection too", async () => {
        setInvokeHandler("search_content", () => {
            throw "upstream returned HTTP 502";
        });

        render(<SearchResults query="Lillyy" site="pornoxo" />);

        expect(
            await screen.findByText(/upstream returned HTTP 502/),
        ).toBeInTheDocument();
    });

    it("still renders the failure heading when the error carries no message", async () => {
        setInvokeHandler("search_content", () => {
            throw { kind: "SearchFailed", data: {} };
        });

        render(<SearchResults query="Lillyy" site="pornoxo" />);

        expect(await screen.findByText("Search failed")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
    });
});
