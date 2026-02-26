// Row grouping and options summary helpers for the format table.

import type { Row } from "@tanstack/react-table";
import type { DownloadOptions, FormatInfo } from "../../types";
import type { FormatGroup } from "../FormatGroupSection";

/** Group sorted TanStack rows into Video+Audio, Video Only, Audio Only sections. */
export function groupRows(rows: Row<FormatInfo>[]): FormatGroup[] {
    const videoAudio: Row<FormatInfo>[] = [];
    const videoOnly: Row<FormatInfo>[] = [];
    const audioOnly: Row<FormatInfo>[] = [];

    for (const row of rows) {
        const f = row.original;
        if (f.has_video && f.has_audio) videoAudio.push(row);
        else if (f.has_video) videoOnly.push(row);
        else audioOnly.push(row);
    }

    const groups: FormatGroup[] = [];
    if (videoAudio.length > 0) groups.push({ label: "Video + Audio", rows: videoAudio });
    if (videoOnly.length > 0) groups.push({ label: "Video Only", rows: videoOnly });
    if (audioOnly.length > 0) groups.push({ label: "Audio Only", rows: audioOnly });
    return groups;
}

/** Build a summary string for the collapsed options trigger. */
export function optionsSummary(opts: DownloadOptions): string {
    const parts: string[] = [];
    if (opts.remux) parts.push(`${opts.remux.toUpperCase()} remux`);
    if (opts.extractAudio) parts.push(`${opts.extractAudio.toUpperCase()} audio`);
    if (opts.subtitles && opts.subtitleLangs.length > 0)
        parts.push(`Subs: ${opts.subtitleLangs.join(", ")}`);
    if (opts.embedThumbnail) parts.push("Thumbnail");
    if (opts.normalizeAudio === true) {
        if (opts.loudnorm) {
            const preset = opts.loudnormPreset ?? "streaming";
            parts.push(`Loudnorm (${preset})`);
        } else {
            parts.push("Peak normalize");
        }
    }
    return parts.length > 0 ? parts.join(" \u00b7 ") : "Default settings";
}
