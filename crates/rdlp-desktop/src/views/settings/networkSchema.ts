// The four timeout-bound schemas (socket/read/download/merge) and
// `poolIdleTimeoutSchema` were deleted here (#585 Task 3). Reason: none has
// a reachable consumer any more. `NetworkSection` now migrates its numeric
// timeout inputs to the shared `NumericField` (React Aria `NumberField`),
// whose `useNumberFieldState.commit()` unconditionally CLAMPS the committed
// value to `[minValue, maxValue]` *before* any validation runs (verified in
// `@react-stately/numberfield`'s `useNumberFieldState.mjs` — see
// `NumericField.tsx`'s doc comment). A zod schema mirroring the same bounds
// can therefore never reject anything a user can actually type through the
// UI — including `poolIdleTimeoutSchema`'s 0-sentinel branch: NumericField's
// own `minValue={1}` on the pool-idle control means a typed `0` clamps to
// `1` before `onCommit` ever fires, so the "use the checkbox" hint text can
// no longer be reached from the UI either. Range enforcement is now
// client-side clamping plus `AppSettings::validate_security()` on the Rust
// side. The 0-sentinel *behaviour* itself is NOT dead — it lives on in
// `formStateToPoolIdleTimeout`/`poolIdleTimeoutToFormState` below, which the
// checkbox still drives.

import type { NetworkRange } from "@/types";

export interface PoolIdleFormState {
    evictIdle: boolean;
    secondsInput: string;
}

/**
 * The stored `pool_idle_timeout` that means "idle eviction disabled" — the
 * engine's sentinel (`EffectiveNetwork::pool_idle_timeout_secs` doc), which the
 * checkbox owns and the numeric control must never produce.
 */
export const POOL_IDLE_DISABLED = 0;

/**
 * The numeric idle-timeout control's lower bound. The owner's range starts at
 * `POOL_IDLE_DISABLED`, but a typed 0 would silently turn eviction off from a
 * control whose visible meaning is "after N seconds"; the control therefore
 * starts one past the sentinel while its upper bound stays the owner's.
 */
export function poolIdleNumericMin(range: NetworkRange): number {
    return Math.max(range.min, POOL_IDLE_DISABLED + 1);
}

export function formStateToPoolIdleTimeout(state: PoolIdleFormState): number | null {
    if (!state.evictIdle) return POOL_IDLE_DISABLED;
    if (state.secondsInput.trim() === "") return null; // use default
    return Number(state.secondsInput);
}

export function poolIdleTimeoutToFormState(value: number | null): PoolIdleFormState {
    if (value === POOL_IDLE_DISABLED) return { evictIdle: false, secondsInput: "" };
    if (value === null) return { evictIdle: true, secondsInput: "" };
    return { evictIdle: true, secondsInput: String(value) };
}
