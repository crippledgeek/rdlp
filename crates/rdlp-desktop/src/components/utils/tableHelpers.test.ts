// Unit tests for tableHelpers.ts — groupRows and optionsSummary.

import { describe, test, expect } from "vitest";
import type { Row } from "@tanstack/react-table";
import type { FormatInfo, DownloadOptions } from "../../types";
import { groupRows, optionsSummary } from "./tableHelpers";
import {
    isCustomNormalization,
    getNormSelectValue,
    handleNormSelectChange,
} from "./normalization";

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

    test("includes 'Custom normalize' when custom target fields are set", () => {
        const result = optionsSummary(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "streaming",
            loudnormTargetI: -16,
        }));
        expect(result).toContain("Custom normalize");
    });

    test("includes 'Custom normalize' when dynamic mode is enabled", () => {
        const result = optionsSummary(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormDynamic: true,
        }));
        expect(result).toContain("Custom normalize");
    });

    test("includes 'Custom normalize' when boost is enabled", () => {
        const result = optionsSummary(makeOptions({
            normalizeAudio: true,
            loudnorm: false,
            normalizeBoost: true,
        }));
        expect(result).toContain("Custom normalize");
    });
});

// ---------------------------------------------------------------------------
// isCustomNormalization
// ---------------------------------------------------------------------------

describe("isCustomNormalization", () => {
    test("returns false when normalizeAudio is null", () => {
        expect(isCustomNormalization(makeOptions())).toBe(false);
    });

    test("returns false when normalizeAudio is false", () => {
        expect(isCustomNormalization(makeOptions({ normalizeAudio: false }))).toBe(false);
    });

    test("returns false for standard peak mode", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            loudnorm: false,
        }))).toBe(false);
    });

    test("returns false for standard loudnorm preset", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "streaming",
        }))).toBe(false);
    });

    test("returns true when loudnormTargetI is set", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormTargetI: -16,
        }))).toBe(true);
    });

    test("returns true when loudnormTargetTp is set", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormTargetTp: -2,
        }))).toBe(true);
    });

    test("returns true when loudnormTargetLra is set", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormTargetLra: 9,
        }))).toBe(true);
    });

    test("returns true when loudnormDynamic is true", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormDynamic: true,
        }))).toBe(true);
    });

    test("returns true when loudnormPrecompress is true", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPrecompress: true,
        }))).toBe(true);
    });

    test("returns true when normalizeBoost is true", () => {
        expect(isCustomNormalization(makeOptions({
            normalizeAudio: true,
            normalizeBoost: true,
        }))).toBe(true);
    });
});

// ---------------------------------------------------------------------------
// getNormSelectValue
// ---------------------------------------------------------------------------

describe("getNormSelectValue", () => {
    test("returns 'default' for null normalizeAudio", () => {
        expect(getNormSelectValue(makeOptions())).toBe("default");
    });

    test("returns 'off' when normalizeAudio is false", () => {
        expect(getNormSelectValue(makeOptions({ normalizeAudio: false }))).toBe("off");
    });

    test("returns 'peak' when normalizeAudio is true and loudnorm is false", () => {
        expect(getNormSelectValue(makeOptions({
            normalizeAudio: true,
            loudnorm: false,
        }))).toBe("peak");
    });

    test("returns 'loudnorm-streaming' for streaming preset", () => {
        expect(getNormSelectValue(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "streaming",
        }))).toBe("loudnorm-streaming");
    });

    test("returns 'loudnorm-streaming' when preset is null", () => {
        expect(getNormSelectValue(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: null,
        }))).toBe("loudnorm-streaming");
    });

    test("returns 'loudnorm-broadcast' for broadcast preset", () => {
        expect(getNormSelectValue(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "broadcast",
        }))).toBe("loudnorm-broadcast");
    });

    test("returns 'loudnorm-loud' for loud preset", () => {
        expect(getNormSelectValue(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "loud",
        }))).toBe("loudnorm-loud");
    });

    test("returns 'custom' when custom target fields are set", () => {
        expect(getNormSelectValue(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormTargetI: -16,
        }))).toBe("custom");
    });

    test("returns 'custom' when boost is enabled", () => {
        expect(getNormSelectValue(makeOptions({
            normalizeAudio: true,
            loudnorm: false,
            normalizeBoost: true,
        }))).toBe("custom");
    });
});

// ---------------------------------------------------------------------------
// handleNormSelectChange
// ---------------------------------------------------------------------------

describe("handleNormSelectChange", () => {
    test("'default' clears all normalization fields to null", () => {
        const result = handleNormSelectChange(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "streaming",
            loudnormTargetI: -16,
        }), "default");
        expect(result.normalizeAudio).toBeNull();
        expect(result.loudnorm).toBeNull();
        expect(result.loudnormPreset).toBeNull();
        expect(result.loudnormTargetI).toBeNull();
        expect(result.normalizeBoost).toBeNull();
    });

    test("'off' sets normalizeAudio to false and clears fields", () => {
        const result = handleNormSelectChange(makeOptions(), "off");
        expect(result.normalizeAudio).toBe(false);
        expect(result.loudnorm).toBe(false);
        expect(result.loudnormDynamic).toBe(false);
        expect(result.normalizeBoost).toBe(false);
    });

    test("'peak' clears custom fields", () => {
        const result = handleNormSelectChange(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormTargetI: -16,
            normalizeBoost: true,
        }), "peak");
        expect(result.normalizeAudio).toBe(true);
        expect(result.loudnorm).toBe(false);
        expect(result.loudnormTargetI).toBeNull();
        expect(result.normalizeBoost).toBeNull();
    });

    test("'custom' enables normalization and keeps existing values", () => {
        const opts = makeOptions({
            normalizeAudio: false,
            loudnorm: false,
            loudnormPreset: "broadcast",
        });
        const result = handleNormSelectChange(opts, "custom");
        expect(result.normalizeAudio).toBe(true);
        expect(result.loudnormPreset).toBe("broadcast");
    });

    test("loudnorm preset clears custom overrides", () => {
        const result = handleNormSelectChange(makeOptions({
            normalizeAudio: true,
            loudnorm: true,
            loudnormTargetI: -16,
            loudnormDynamic: true,
            normalizeBoost: true,
            normalizeBoostDb: 8,
        }), "loudnorm-broadcast");
        expect(result.normalizeAudio).toBe(true);
        expect(result.loudnorm).toBe(true);
        expect(result.loudnormPreset).toBe("broadcast");
        expect(result.loudnormTargetI).toBeNull();
        expect(result.loudnormDynamic).toBeNull();
        expect(result.normalizeBoost).toBeNull();
        expect(result.normalizeBoostDb).toBeNull();
    });
});
