// Pure function tests for normalization helpers — no DOM rendering needed.

import { getNormSelectValue, handleNormSelectChange } from "./normalization";
import { activePreset, PRESETS } from "../FormatOptionsPanel";
import type { DownloadOptions } from "../../types";

const base: DownloadOptions = {
    format: null,
    outputDir: null,
    subtitles: false,
    subtitleLangs: [],
    remux: null,
    extractAudio: null,
    embedThumbnail: true,
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
    embedSubtitles: null,
    videoEncoder: null,
};

// ---------------------------------------------------------------------------
// getNormSelectValue
// ---------------------------------------------------------------------------

describe("getNormSelectValue", () => {
    it.each([
        { name: "null → 'default'", props: { normalizeAudio: null as null }, expected: "default" },
        { name: "false → 'off'", props: { normalizeAudio: false, loudnorm: false }, expected: "off" },
        { name: "peak mode", props: { normalizeAudio: true, loudnorm: false }, expected: "peak" },
        { name: "streaming preset", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: "streaming" }, expected: "loudnorm-streaming" },
        { name: "broadcast preset", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: "broadcast" }, expected: "loudnorm-broadcast" },
        { name: "loud preset", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: "loud" }, expected: "loudnorm-loud" },
        { name: "null preset defaults to streaming", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: null as null }, expected: "loudnorm-streaming" },
    ])("$name", ({ props, expected }) => {
        expect(getNormSelectValue({ ...base, ...props })).toBe(expected);
    });
});

// ---------------------------------------------------------------------------
// handleNormSelectChange
// ---------------------------------------------------------------------------

describe("handleNormSelectChange", () => {
    it("'default' nulls all 10 normalization fields", () => {
        const result = handleNormSelectChange({ ...base, normalizeAudio: false, loudnorm: false }, "default");
        expect(result.normalizeAudio).toBeNull();
        expect(result.loudnorm).toBeNull();
        expect(result.loudnormPreset).toBeNull();
        expect(result.loudnormTargetI).toBeNull();
        expect(result.loudnormTargetTp).toBeNull();
        expect(result.loudnormTargetLra).toBeNull();
        expect(result.loudnormDynamic).toBeNull();
        expect(result.loudnormPrecompress).toBeNull();
        expect(result.normalizeBoost).toBeNull();
        expect(result.normalizeBoostDb).toBeNull();
    });

    it("'off' sets normalizeAudio false and loudnorm false", () => {
        const result = handleNormSelectChange({ ...base, normalizeAudio: null }, "off");
        expect(result.normalizeAudio).toBe(false);
        expect(result.loudnorm).toBe(false);
        expect(result.normalizeBoost).toBe(false);
        expect(result.normalizeBoostDb).toBeNull();
    });

    it("'peak' sets normalizeAudio true and loudnorm false", () => {
        const result = handleNormSelectChange({ ...base, normalizeAudio: null }, "peak");
        expect(result.normalizeAudio).toBe(true);
        expect(result.loudnorm).toBe(false);
        expect(result.loudnormPreset).toBeNull();
    });

    it("'loudnorm-streaming' sets loudnorm true and preset 'streaming'", () => {
        const result = handleNormSelectChange({ ...base, normalizeAudio: null }, "loudnorm-streaming");
        expect(result.normalizeAudio).toBe(true);
        expect(result.loudnorm).toBe(true);
        expect(result.loudnormPreset).toBe("streaming");
    });

    it("'loudnorm-broadcast' sets loudnorm true and preset 'broadcast'", () => {
        const result = handleNormSelectChange({ ...base, normalizeAudio: null }, "loudnorm-broadcast");
        expect(result.normalizeAudio).toBe(true);
        expect(result.loudnorm).toBe(true);
        expect(result.loudnormPreset).toBe("broadcast");
    });
});

// ---------------------------------------------------------------------------
// activePreset
// ---------------------------------------------------------------------------

describe("activePreset", () => {
    it("null format returns null", () => {
        expect(activePreset(null)).toBeNull();
    });

    it("matching selector returns preset id", () => {
        const best = PRESETS.find((p) => p.id === "best")!;
        expect(activePreset(best.selector)).toBe("best");
    });

    it("720p preset matches", () => {
        const preset = PRESETS.find((p) => p.id === "720p")!;
        expect(activePreset(preset.selector)).toBe("720p");
    });

    it("1080p preset matches", () => {
        const preset = PRESETS.find((p) => p.id === "1080p")!;
        expect(activePreset(preset.selector)).toBe("1080p");
    });

    it("unrecognized selector returns null", () => {
        expect(activePreset("custom-selector")).toBeNull();
    });
});
