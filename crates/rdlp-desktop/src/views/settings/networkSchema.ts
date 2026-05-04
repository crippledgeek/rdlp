import { z } from "zod";

const intInRange = (min: number, max: number, label: string) =>
    z.preprocess(
        (v) => (v === "" || v === null || v === undefined ? null : Number(v)),
        z.union([
            z.null(),
            z.number({ invalid_type_error: `${label} must be a number` })
                .int(`${label} must be an integer`)
                .min(min, `${label} must be ≥ ${min}`)
                .max(max, `${label} must be ≤ ${max}`),
        ]),
    );

export const socketTimeoutSchema = intInRange(1, 300, "Connection timeout");
export const readTimeoutSchema = intInRange(1, 600, "Read timeout");

// Numeric input alone — the 0-sentinel is owned by the checkbox.
// Reject 0 explicitly so users who type 0 see a hint to use the checkbox.
export const poolIdleTimeoutSchema = z.preprocess(
    (v) => (v === "" || v === null || v === undefined ? null : Number(v)),
    z.union([
        z.null(),
        z.number({ invalid_type_error: "Idle timeout must be a number" })
            .superRefine((n, ctx) => {
                if (!Number.isInteger(n)) {
                    ctx.addIssue({
                        code: z.ZodIssueCode.custom,
                        message: "Idle timeout must be an integer",
                    });
                    return;
                }
                if (n === 0) {
                    ctx.addIssue({
                        code: z.ZodIssueCode.custom,
                        message: "Use the checkbox to keep connections alive forever",
                    });
                    return;
                }
                if (n < 1) {
                    ctx.addIssue({
                        code: z.ZodIssueCode.custom,
                        message: "Idle timeout must be ≥ 1",
                    });
                    return;
                }
                if (n > 3600) {
                    ctx.addIssue({
                        code: z.ZodIssueCode.custom,
                        message: "Idle timeout must be ≤ 3600",
                    });
                    return;
                }
            }),
    ]),
);

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
