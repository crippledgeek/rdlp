// Shared job lifecycle state transitions for the downloads cache.

import type { DownloadJob } from "../types";

/**
 * The field patch applied when a job reaches the terminal `cancelled` state:
 * sets `status` and clears every transient progress field.
 *
 * Single source of truth used by BOTH cancel paths — the optimistic flip in
 * `api/downloads.cancelDownload` and the `download-cancelled` event reducer in
 * `events/registerDownloadEvents`. They MUST stay in lock-step; a field added
 * to one path but not the other would desync the optimistic and confirmed
 * cancel state. Spread over the existing job: `{ ...job, ...cancelledJobPatch() }`.
 */
export function cancelledJobPatch(): Partial<DownloadJob> {
    return {
        status: "cancelled",
        statusMessage: null,
        currentUnit: null,
        speed: null,
        eta: null,
        progress: null,
    };
}
