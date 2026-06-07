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


/**
 * Placeholder text rendered in the GUI when an extractor produces an
 * empty/null/whitespace-only title. Single source of truth so JobCard,
 * MediaHero, HistoryView etc. don't drift apart on labelling.
 *
 * Surfaced by the godresource port: API returned `title: null`, the
 * shim's `_real_extract` wrote `'title': ''`, and the backend's
 * template renderer (after the empty-string-as-missing fix) maps that
 * to either the `|default` pipe or "NA". For the UI we use the same
 * placeholder text everywhere so users see consistent language.
 */
export const TITLE_PLACEHOLDER = "Unknown";

/**
 * Normalise a title-like string for display. Returns `TITLE_PLACEHOLDER`
 * when the input is null, undefined, empty, or whitespace-only —
 * matching the backend's empty-string-as-missing semantics. Use at
 * every render site that displays a video/playlist title.
 *
 * @param title - The raw title from an `InfoDict` / job state.
 * @returns The trimmed title if non-blank, otherwise `TITLE_PLACEHOLDER`.
 */
export function displayTitle(title: string | null | undefined): string {
    if (title == null) return TITLE_PLACEHOLDER;
    const trimmed = title.trim();
    return trimmed.length === 0 ? TITLE_PLACEHOLDER : trimmed;
}

/** Convert a 0–1 progress fraction to a rounded whole-percent number. */
export function progressPercent(fraction: number | null | undefined): number {
    return Math.round((fraction ?? 0) * 100);
}

/**
 * Format a 0–1 progress fraction as a percent string. In-flight post-processing
 * (e.g. a slow recode) is shown with one decimal so movement below 1% is legible;
 * all other states use a whole percent to avoid display churn.
 */
export function progressPercentLabel(
    fraction: number | null | undefined,
    isProcessing: boolean,
): string {
    if (isProcessing) {
        return ((fraction ?? 0) * 100).toFixed(1);
    }
    return String(progressPercent(fraction));
}
