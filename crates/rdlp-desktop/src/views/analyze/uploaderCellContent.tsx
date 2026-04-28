// Pure render helper for the uploader cell's inner content.
//
// Extracted from SearchResultRow so the four enrichment states
// (loaded / loading / enriched-empty / pre-enrichment-fallthrough) can
// be unit-tested without a TanStack Table + Tauri IPC harness.

import type { ReactNode } from "react";

export interface UploaderCellInputs {
    /** The (possibly enriched) uploader value for the row. */
    uploader: string | null | undefined;
    /** True while the per-row `enrich_search_result` Tauri call is in flight. */
    isEnriching: boolean;
    /** True when enrichment has resolved (success or with a null uploader). */
    enrichmentResolved: boolean;
}

export type UploaderCellState =
    | { kind: "loaded"; content: ReactNode }
    | { kind: "loading" }
    | { kind: "unavailable" }
    | { kind: "fallthrough" };

/**
 * Decide what to render for the uploader cell.
 *
 * Order matters — `loaded` wins over `loading` (we keep showing the value
 * while a refetch is happening), and `unavailable` wins over
 * `fallthrough` (only fall through before enrichment has been attempted).
 */
export function uploaderCellState(inputs: UploaderCellInputs): UploaderCellState {
    if (inputs.uploader != null && inputs.uploader !== "") {
        return { kind: "loaded", content: inputs.uploader };
    }
    if (inputs.isEnriching) {
        return { kind: "loading" };
    }
    if (inputs.enrichmentResolved) {
        return { kind: "unavailable" };
    }
    return { kind: "fallthrough" };
}
