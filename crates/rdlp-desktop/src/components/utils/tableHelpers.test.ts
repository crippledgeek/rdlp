// Unit tests for tableHelpers.ts — groupRows and optionsSummary.

import { describe, test, expect } from "vitest";
import type { Row } from "@tanstack/react-table";
import type { FormatInfo, DownloadOptions } from "../../types";
import { groupRows, optionsSummary } from "./tableHelpers";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Create a minimal Row<FormatInfo> stub — only `original` is needed by groupRows. */
function makeRow(overrides: Partial<FormatInfo>): Row<FormatInfo> {
    const format: FormatInfo = {
        format_id: "id",
        ext: "mp4",
        format_note: null,
        width: null,
        height: null,
        fps: null,
        tbr: null,
        vcodec: null,
        acodec: null,
        filesize: null,
        vbr: null,
        abr: null,
        asr: null,
        protocol: "https",
        has_video: true,
        has_audio: true,
        ...overrides,
    };
    return { original: format, id: format.format_id } as unknown as Row<FormatInfo>;
}

function makeOptions(overrides: Partial<DownloadOptions> = {}): DownloadOptions {
    return {
        format: null,
        outputDir: null,
        subtitles: false,
        subtitleLangs: [],
        remux: null,
        extractAudio: null,
        embedThumbnail: false,
        audioMultistreams: false,
        recodeVideo: null,
        normalizeAudio: null,
        loudnorm: null,
        loudnormPreset: null,
        loudnormTargetI: null,
        loudnormTargetTp: null,
        loudnormTargetLra: null,
        loudnormDynamic: null,
        loudnormPrecompress: null,
        normalizeBoost: null,
        normalizeBoostDb: null,
        ...overrides,
    };
}

// ---------------------------------------------------------------------------
// groupRows
// ---------------------------------------------------------------------------

describe("groupRows", () => {
    test("returns empty array for empty input", () => {
        expect(groupRows([])).toEqual([]);
    });

    test("places video+audio rows in first group", () => {
        const rows = [makeRow({ has_video: true, has_audio: true })];
        const groups = groupRows(rows);
        expect(groups).toHaveLength(1);
        expect(groups[0].label).toBe("Video + Audio");
        expect(groups[0].rows).toHaveLength(1);
    });

    test("places video-only rows in second group", () => {
        const rows = [makeRow({ has_video: true, has_audio: false })];
        const groups = groupRows(rows);
        expect(groups).toHaveLength(1);
        expect(groups[0].label).toBe("Video Only");
    });

    test("places audio-only rows in third group", () => {
        const rows = [makeRow({ has_video: false, has_audio: false })];
        const groups = groupRows(rows);
        expect(groups).toHaveLength(1);
        expect(groups[0].label).toBe("Audio Only");
    });

    test("produces correct group ordering with all three types", () => {
        const rows = [
            makeRow({ has_video: false, has_audio: false }),
            makeRow({ has_video: true, has_audio: false }),
            makeRow({ has_video: true, has_audio: true }),
        ];
        const groups = groupRows(rows);
        expect(groups).toHaveLength(3);
        expect(groups[0].label).toBe("Video + Audio");
        expect(groups[1].label).toBe("Video Only");
        expect(groups[2].label).toBe("Audio Only");
    });

    test("omits empty groups", () => {
        const rows = [makeRow({ has_video: true, has_audio: true })];
        const groups = groupRows(rows);
        // Only V+A group should exist — no Video Only or Audio Only
        expect(groups.every((g) => g.label !== "Video Only")).toBe(true);
        expect(groups.every((g) => g.label !== "Audio Only")).toBe(true);
    });
});

// ---------------------------------------------------------------------------
// optionsSummary
// ---------------------------------------------------------------------------

describe("optionsSummary", () => {
    test("returns 'Default settings' when no options are set", () => {
        expect(optionsSummary(makeOptions())).toBe("Default settings");
    });

    test("includes remux format in uppercase", () => {
        const result = optionsSummary(makeOptions({ remux: "mp4" }));
        expect(result).toContain("MP4 remux");
    });

    test("includes audio format in uppercase", () => {
        const result = optionsSummary(makeOptions({ extractAudio: "mp3" }));
        expect(result).toContain("MP3 audio");
    });

    test("includes subtitle languages when subtitles enabled", () => {
        const result = optionsSummary(makeOptions({ subtitles: true, subtitleLangs: ["en", "fr"] }));
        expect(result).toContain("Subs: en, fr");
    });

    test("does not include subtitle entry when langs array is empty", () => {
        const result = optionsSummary(makeOptions({ subtitles: true, subtitleLangs: [] }));
        expect(result).not.toContain("Subs");
    });

    test("includes 'Thumbnail' when embedThumbnail is true", () => {
        const result = optionsSummary(makeOptions({ embedThumbnail: true }));
        expect(result).toContain("Thumbnail");
    });

    test("joins multiple options with middle-dot separator", () => {
        const result = optionsSummary(makeOptions({ remux: "mkv", embedThumbnail: true }));
        expect(result).toContain("\u00b7");
    });

    test("includes 'Peak normalize' when normalizeAudio is true and loudnorm is false", () => {
        const result = optionsSummary(makeOptions({ normalizeAudio: true, loudnorm: false }));
        expect(result).toContain("Peak normalize");
    });

    test("includes 'Loudnorm (streaming)' when loudnorm is enabled with streaming preset", () => {
        const result = optionsSummary(makeOptions({ normalizeAudio: true, loudnorm: true, loudnormPreset: "streaming" }));
        expect(result).toContain("Loudnorm (streaming)");
    });

    test("includes 'Loudnorm (broadcast)' when loudnorm is enabled with broadcast preset", () => {
        const result = optionsSummary(makeOptions({ normalizeAudio: true, loudnorm: true, loudnormPreset: "broadcast" }));
        expect(result).toContain("Loudnorm (broadcast)");
    });

    test("defaults to 'Loudnorm (streaming)' when loudnorm preset is null", () => {
        const result = optionsSummary(makeOptions({ normalizeAudio: true, loudnorm: true, loudnormPreset: null }));
        expect(result).toContain("Loudnorm (streaming)");
    });

    test("does not include normalization when normalizeAudio is null", () => {
        const result = optionsSummary(makeOptions({ normalizeAudio: null }));
        expect(result).not.toContain("normalize");
        expect(result).not.toContain("Loudnorm");
    });
});
