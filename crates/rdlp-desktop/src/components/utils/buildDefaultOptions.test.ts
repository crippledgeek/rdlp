// @vitest-environment node
// Pure function tests for buildDefaultOptions — no DOM needed.

import { describe, it, expect } from "vitest";
import { buildDefaultOptions } from "../FormatOptionsPanel";
import type { AppSettings } from "../../types";

const fullSettings: AppSettings = {
    output_dir: "/home/user/Videos",
    default_remux: null,
    default_extract_audio: null,
    default_subtitle_format: null,
    default_subtitle_langs: [],
    embed_thumbnail: true,
    write_thumbnail: false,
    embed_metadata: true,
    verbose: false,
    default_search_provider: null,
    output_template: null,
    cookies_from_browser: null,
    cookies_file: null,
    proxy: null,
    rate_limit: null,
    normalize_audio: true,
    audio_gain_target: null,
    loudnorm: true,
    loudnorm_preset: "streaming",
    loudnorm_target_i: -14,
    loudnorm_target_tp: -1,
    loudnorm_target_lra: 11,
    loudnorm_dynamic: true,
    loudnorm_precompress: false,
    normalize_boost: true,
    normalize_boost_db: 8,
    embed_subtitles: false,
};

describe("buildDefaultOptions", () => {
    it("returns sensible defaults when settings is null", () => {
        const opts = buildDefaultOptions(null);
        expect(opts.format).toBeNull();
        expect(opts.subtitles).toBe(false);
        expect(opts.embedThumbnail).toBe(true);
        expect(opts.normalizeAudio).toBe(false);
        expect(opts.loudnorm).toBe(false);
        expect(opts.loudnormPreset).toBeNull();
        expect(opts.loudnormTargetI).toBeNull();
        expect(opts.loudnormTargetTp).toBeNull();
        expect(opts.loudnormTargetLra).toBeNull();
        expect(opts.loudnormDynamic).toBe(false);
        expect(opts.loudnormPrecompress).toBe(false);
        expect(opts.normalizeBoost).toBe(false);
        expect(opts.normalizeBoostDb).toBeNull();
    });

    it("populates all 10 normalization fields from settings when normalization is enabled", () => {
        const opts = buildDefaultOptions(fullSettings);
        expect(opts.normalizeAudio).toBe(true);
        expect(opts.loudnorm).toBe(true);
        expect(opts.loudnormPreset).toBe("streaming");
        expect(opts.loudnormTargetI).toBe(-14);
        expect(opts.loudnormTargetTp).toBe(-1);
        expect(opts.loudnormTargetLra).toBe(11);
        expect(opts.loudnormDynamic).toBe(true);
        expect(opts.loudnormPrecompress).toBe(false);
        expect(opts.normalizeBoost).toBe(true);
        expect(opts.normalizeBoostDb).toBe(8);
    });

    it("maps normalize_audio true with loudnorm false as peak mode", () => {
        const settings: AppSettings = {
            ...fullSettings,
            normalize_audio: true,
            loudnorm: false,
            loudnorm_preset: null,
        };
        const opts = buildDefaultOptions(settings);
        expect(opts.normalizeAudio).toBe(true);
        expect(opts.loudnorm).toBe(false);
        expect(opts.loudnormPreset).toBeNull();
    });

    it("respects embed_thumbnail from settings", () => {
        const settings: AppSettings = { ...fullSettings, embed_thumbnail: false };
        const opts = buildDefaultOptions(settings);
        expect(opts.embedThumbnail).toBe(false);
    });

    it("sets subtitles true when default_subtitle_langs is non-empty", () => {
        const settings: AppSettings = {
            ...fullSettings,
            default_subtitle_langs: ["en", "sv"],
        };
        const opts = buildDefaultOptions(settings);
        expect(opts.subtitles).toBe(true);
        expect(opts.subtitleLangs).toEqual(["en", "sv"]);
    });

    it("always sets format to null regardless of settings", () => {
        const opts = buildDefaultOptions(fullSettings);
        expect(opts.format).toBeNull();
    });
});
