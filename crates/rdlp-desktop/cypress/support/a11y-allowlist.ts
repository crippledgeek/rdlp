// Per-page accessibility violation allowlist.
//
// Each entry documents a known violation, why it is currently waived, and the
// GitHub issue tracking the fix. Entries shrink commit-by-commit during the
// a11y sprint. The final empty file is the goal.
//
// Usage:
//   import { allowlistFor } from "../support/a11y-allowlist";
//   cy.checkA11y(undefined, {
//     runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21aa"] },
//     rules: allowlistFor("queue-empty"),
//   });

export type PageId =
    | "search-empty"
    | "search-results"
    | "queue-empty"
    | "queue-active"
    | "history"
    | "settings"
    | "format-dialog"
    | "logs-expanded";

export interface AllowlistEntry {
    rule: string;
    reason: string;
    issue: string;
}

const ENTRIES: Record<PageId, AllowlistEntry[]> = {
    "search-empty": [],
    "search-results": [],
    "queue-empty": [],
    "queue-active": [],
    "history": [],
    "settings": [],
    "format-dialog": [],
    "logs-expanded": [],
};

/**
 * Build a `rules` map for `cy.checkA11y()` that disables only the rules
 * named in the allowlist for the given page. All other rules remain active.
 */
export function allowlistFor(page: PageId): Record<string, { enabled: false }> {
    const out: Record<string, { enabled: false }> = {};
    for (const entry of ENTRIES[page]) {
        out[entry.rule] = { enabled: false };
    }
    return out;
}
