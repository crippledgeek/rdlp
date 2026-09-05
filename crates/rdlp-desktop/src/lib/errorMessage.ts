/**
 * Extract a human-readable message from an unknown thrown value.
 *
 * A Tauri `invoke` rejection arrives as a plain object carrying the command's
 * serialized error, not as an `Error`, so `String(err)` on one yields
 * "[object Object]". Every call site that reports a failure to the user needs
 * the same three-way unwrap, which is why it lives here rather than being
 * written out at each one.
 *
 * @param err - The caught value.
 * @param fallback - Message to use when nothing readable can be extracted.
 */
export function errorMessage(err: unknown, fallback: string): string {
    if (err instanceof Error) return err.message;
    if (typeof err === "object" && err !== null && "message" in err) {
        return String((err as { message: unknown }).message);
    }
    if (typeof err === "string" && err.length > 0) return err;
    return fallback;
}
