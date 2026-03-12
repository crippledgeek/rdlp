// Pure function tests for settings validation — no DOM rendering needed.

import { validateSettings } from "./SettingsPage";
import type { AppSettings } from "../types";

const validSettings: AppSettings = {
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
    normalize_audio: false,
    audio_gain_target: null,
    loudnorm: false,
    loudnorm_preset: null,
    loudnorm_target_i: null,
    loudnorm_target_tp: null,
    loudnorm_target_lra: null,
    loudnorm_dynamic: false,
    loudnorm_precompress: false,
    normalize_boost: false,
    normalize_boost_db: null,
    embed_subtitles: false,
};

describe("validateSettings", () => {
    it("returns null for valid settings", () => {
        expect(validateSettings(validSettings)).toBeNull();
    });

    it("rejects loudnorm_target_i above 0", () => {
        const result = validateSettings({ ...validSettings, loudnorm_target_i: 5 });
        expect(result).toMatch(/loudness target/i);
    });

    it("rejects loudnorm_target_i below -70", () => {
        const result = validateSettings({ ...validSettings, loudnorm_target_i: -80 });
        expect(result).toMatch(/loudness target/i);
    });

    it("accepts loudnorm_target_i at -14", () => {
        expect(validateSettings({ ...validSettings, loudnorm_target_i: -14 })).toBeNull();
    });

    it("rejects loudnorm_target_tp above 0", () => {
        const result = validateSettings({ ...validSettings, loudnorm_target_tp: 1 });
        expect(result).toMatch(/true peak/i);
    });

    it("rejects loudnorm_target_tp below -9", () => {
        const result = validateSettings({ ...validSettings, loudnorm_target_tp: -10 });
        expect(result).toMatch(/true peak/i);
    });

    it("rejects normalize_boost_db above 30", () => {
        const result = validateSettings({ ...validSettings, normalize_boost_db: 50 });
        expect(result).toMatch(/boost gain/i);
    });

    it("rejects normalize_boost_db below 0", () => {
        const result = validateSettings({ ...validSettings, normalize_boost_db: -1 });
        expect(result).toMatch(/boost gain/i);
    });

    it("rejects loudnorm_target_lra below 1", () => {
        const result = validateSettings({ ...validSettings, loudnorm_target_lra: 0 });
        expect(result).toMatch(/loudness range/i);
    });

    it("rejects loudnorm_target_lra above 30", () => {
        const result = validateSettings({ ...validSettings, loudnorm_target_lra: 31 });
        expect(result).toMatch(/loudness range/i);
    });
});
