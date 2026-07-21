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

export interface PoolIdleFormState {
    evictIdle: boolean;
    secondsInput: string;
}

export function formStateToPoolIdleTimeout(state: PoolIdleFormState): number | null {
    if (!state.evictIdle) return 0; // sentinel: disable eviction
    if (state.secondsInput.trim() === "") return null; // use default
    return Number(state.secondsInput);
}

export function poolIdleTimeoutToFormState(value: number | null): PoolIdleFormState {
    if (value === 0) return { evictIdle: false, secondsInput: "" };
    if (value === null) return { evictIdle: true, secondsInput: "" };
    return { evictIdle: true, secondsInput: String(value) };
}
