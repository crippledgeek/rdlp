import { render, screen, fireEvent } from "@/test/test-utils";
import {
    FormatOptionsPanel,
    buildDefaultOptions,
    PRESETS,
} from "./FormatOptionsPanel";
import type { AppSettings, DownloadOptions } from "../types";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Minimal valid AppSettings with all normalization fields present. */
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

/** Default DownloadOptions used in rendering tests. */
const defaultOptions: DownloadOptions = {
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
};

function renderPanel(value: DownloadOptions, onChange = vi.fn()) {
    return render(
        <FormatOptionsPanel value={value} onChange={onChange} />,
    );
}

// ---------------------------------------------------------------------------
// A. buildDefaultOptions — pure function unit tests
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// B. Quality preset radio buttons (rendering required — RadioGroup DOM)
// ---------------------------------------------------------------------------

describe("Quality preset radio buttons", () => {
    it("clicking a preset calls onChange with the preset's selector string", () => {
        const onChange = vi.fn();
        renderPanel(defaultOptions, onChange);
        fireEvent.click(screen.getByRole("radio", { name: /720p/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        const preset = PRESETS.find((p) => p.id === "720p")!;
        expect(called.format).toBe(preset.selector);
    });

    it("presets are hidden when hidePresets is true", () => {
        render(
            <FormatOptionsPanel
                value={defaultOptions}
                onChange={vi.fn()}
                hidePresets
            />,
        );
        expect(screen.queryByRole("radio")).not.toBeInTheDocument();
    });
});

// ---------------------------------------------------------------------------
// C. Remux and Extract Audio selects (rendering required — Radix Select DOM)
// ---------------------------------------------------------------------------

describe("Remux Select", () => {
    function getRemuxSelect() {
        const label = screen.getByText(/^remux$/i);
        const container = label.closest("div.flex-col")!;
        return container.querySelector('[role="combobox"]') as HTMLElement;
    }

    it("shows 'None' when remux is null and selecting MP4 calls onChange", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, remux: null }, onChange);
        expect(getRemuxSelect()).toHaveTextContent(/^none$/i);
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mp4$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.remux).toBe("mp4");
    });
});

describe("Extract Audio Select", () => {
    function getAudioSelect() {
        const label = screen.getByText(/extract audio/i);
        const container = label.closest("div.flex-col")!;
        return container.querySelector('[role="combobox"]') as HTMLElement;
    }

    it("shows 'None' when extractAudio is null and selecting MP3 calls onChange", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, extractAudio: null }, onChange);
        expect(getAudioSelect()).toHaveTextContent(/^none$/i);
        fireEvent.pointerDown(getAudioSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mp3$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.extractAudio).toBe("mp3");
    });
});

// ---------------------------------------------------------------------------
// D. Audio Normalization Select — wiring smoke test (1 render)
// Logic is tested in normalization.test.ts as pure functions.
// ---------------------------------------------------------------------------

describe("Audio Normalization Select — integration", () => {
    it("renders correct value and wires onChange through to normalization helper", () => {
        const onChange = vi.fn();
        renderPanel(
            { ...defaultOptions, normalizeAudio: false, loudnorm: false },
            onChange,
        );
        const label = screen.getByText(/audio normalization/i);
        const container = label.closest("div.flex-col")!;
        const trigger = container.querySelector('[role="combobox"]') as HTMLElement;
        expect(trigger).toHaveTextContent(/^off$/i);
        fireEvent.pointerDown(trigger, { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /use settings default/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.normalizeAudio).toBeNull();
    });
});
