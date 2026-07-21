// Bytes ↔ MiB conversion for byte-valued settings.
//
// Bytes are the single source of truth: they are what `AppSettings` persists and what
// `Config` consumes. MiB is a DISPLAY PROJECTION only — never stored, and never written
// back into storage for a field the user did not actually edit. Re-multiplying a rounded
// display value silently drifts a hand-edited byte value (see the lossy-direction test).
//
// IEC 80000-13 binary prefix: 1 MiB = 1024² bytes.

export const BYTES_PER_MIB = 1_048_576;

/** Bytes → whole MiB for display. Lossy: do not use to reconstruct a stored value. */
export function bytesToMibDisplay(bytes: number): number {
    return Math.round(bytes / BYTES_PER_MIB);
}

/** Whole MiB (user input) → exact bytes for storage. */
export function mibDisplayToBytes(mib: number): number {
    return Math.round(mib * BYTES_PER_MIB);
}
