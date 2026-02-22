import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
    return twMerge(clsx(inputs));
}

/**
 * Parse an upload timestamp string into a Date, handling multiple formats:
 * - Unix timestamp (9-10 digit number string) -> seconds since epoch
 * - YYYYMMDD (exactly 8 digits) -> date-only
 * - "YYYY-MM-DD" or "YYYY-MM-DD HH:MM:SS" -> ISO-like date string
 * Returns null if the value is unparseable or results in an invalid date.
 */
export function parseUploadTimestamp(value: string): Date | null {
    const trimmed = value.trim();
    if (trimmed === "") return null;

    // Unix timestamp: 9-10 digit number (seconds since epoch)
    if (/^\d{9,10}$/.test(trimmed)) {
        const d = new Date(Number(trimmed) * 1000);
        return Number.isNaN(d.getTime()) ? null : d;
    }

    // YYYYMMDD: exactly 8 digits
    if (/^\d{8}$/.test(trimmed)) {
        const y = trimmed.slice(0, 4);
        const m = trimmed.slice(4, 6);
        const day = trimmed.slice(6, 8);
        const d = new Date(`${y}-${m}-${day}T00:00:00`);
        return Number.isNaN(d.getTime()) ? null : d;
    }

    // "YYYY-MM-DD" or "YYYY-MM-DD HH:MM:SS"
    if (/^\d{4}-\d{2}-\d{2}(\s\d{2}:\d{2}:\d{2})?$/.test(trimmed)) {
        const normalized = trimmed.replace(" ", "T");
        const d = new Date(normalized);
        return Number.isNaN(d.getTime()) ? null : d;
    }

    return null;
}
