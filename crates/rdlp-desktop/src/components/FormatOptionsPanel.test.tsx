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
// B. activePreset — tested indirectly via rendered RadioGroup selection
// (also tested as a pure function by inspecting PRESETS)
// ---------------------------------------------------------------------------

describe("activePreset (via PRESETS constant)", () => {
    it("null format has no matching preset", () => {
        // Render with format: null → no radio should appear checked
        const { container } = renderPanel({ ...defaultOptions, format: null });
        const radios = container.querySelectorAll('[role="radio"]');
        for (const r of radios) {
            expect(r).toHaveAttribute("aria-checked", "false");
        }
    });

    it("format matching a preset selector selects that preset", () => {
        const bestPreset = PRESETS.find((p) => p.id === "best")!;
        renderPanel({ ...defaultOptions, format: bestPreset.selector });
        const radio = screen.getByRole("radio", { name: /best quality/i });
        expect(radio).toHaveAttribute("aria-checked", "true");
    });

    it("format matching 720p preset selects 720p", () => {
        const preset = PRESETS.find((p) => p.id === "720p")!;
        renderPanel({ ...defaultOptions, format: preset.selector });
        const radio = screen.getByRole("radio", { name: /720p/i });
        expect(radio).toHaveAttribute("aria-checked", "true");
    });

    it("unrecognized format selector does not select any preset", () => {
        const { container } = renderPanel({
            ...defaultOptions,
            format: "custom-selector",
        });
        const radios = container.querySelectorAll('[role="radio"]');
        for (const r of radios) {
            expect(r).toHaveAttribute("aria-checked", "false");
        }
    });
});

// ---------------------------------------------------------------------------
// C. Audio Normalization Select value derivation
// ---------------------------------------------------------------------------

describe("Audio Normalization Select — displayed value", () => {
    /** Helper: get the combobox for Audio Normalization. */
    function getNormSelect() {
        const label = screen.getByText(/audio normalization/i);
        const container = label.closest("div.flex-col")!;
        return container.querySelector('[role="combobox"]') as HTMLElement;
    }

    // Each case needs its own render (different props), but we use test.each
    // to share boilerplate and reduce per-test overhead.
    it.each([
        { name: "normalizeAudio null shows 'Use Settings Default'", props: { normalizeAudio: null as null }, expected: /use settings default/i },
        { name: "normalizeAudio false shows 'Off'", props: { normalizeAudio: false, loudnorm: false }, expected: /^off$/i },
        { name: "normalizeAudio true, loudnorm false shows 'Peak'", props: { normalizeAudio: true, loudnorm: false }, expected: /^peak$/i },
        { name: "loudnorm streaming", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: "streaming" }, expected: /loudnorm.*streaming/i },
        { name: "loudnorm broadcast", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: "broadcast" }, expected: /loudnorm.*broadcast/i },
        { name: "loudnorm loud", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: "loud" }, expected: /loudnorm.*loud/i },
        { name: "preset null defaults to streaming", props: { normalizeAudio: true, loudnorm: true, loudnormPreset: null as null }, expected: /loudnorm.*streaming/i },
    ])("$name", ({ props, expected }) => {
        renderPanel({ ...defaultOptions, ...props });
        expect(getNormSelect()).toHaveTextContent(expected);
    });
});

// ---------------------------------------------------------------------------
// D. Audio Normalization Select onValueChange branches
// ---------------------------------------------------------------------------

describe("Audio Normalization Select — onChange branches", () => {
    function openNormSelect() {
        const label = screen.getByText(/audio normalization/i);
        const container = label.closest("div.flex-col")!;
        const trigger = container.querySelector('[role="combobox"]') as HTMLElement;
        fireEvent.pointerDown(trigger, { button: 0, pointerType: "mouse" });
    }

    it("selecting 'default' calls onChange with all 10 normalization fields null", () => {
        const onChange = vi.fn();
        renderPanel(
            { ...defaultOptions, normalizeAudio: false, loudnorm: false },
            onChange,
        );
        openNormSelect();
        fireEvent.click(screen.getByRole("option", { name: /use settings default/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.normalizeAudio).toBeNull();
        expect(called.loudnorm).toBeNull();
        expect(called.loudnormPreset).toBeNull();
        expect(called.loudnormTargetI).toBeNull();
        expect(called.loudnormTargetTp).toBeNull();
        expect(called.loudnormTargetLra).toBeNull();
        expect(called.loudnormDynamic).toBeNull();
        expect(called.loudnormPrecompress).toBeNull();
        expect(called.normalizeBoost).toBeNull();
        expect(called.normalizeBoostDb).toBeNull();
    });

    it("selecting 'off' calls onChange with normalizeAudio false and loudnorm false", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, normalizeAudio: null }, onChange);
        openNormSelect();
        fireEvent.click(screen.getByRole("option", { name: /^off$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.normalizeAudio).toBe(false);
        expect(called.loudnorm).toBe(false);
        expect(called.normalizeBoost).toBe(false);
        expect(called.normalizeBoostDb).toBeNull();
    });

    it("selecting 'peak' calls onChange with normalizeAudio true and loudnorm false", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, normalizeAudio: null }, onChange);
        openNormSelect();
        fireEvent.click(screen.getByRole("option", { name: /^peak$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.normalizeAudio).toBe(true);
        expect(called.loudnorm).toBe(false);
        expect(called.loudnormPreset).toBeNull();
    });

    it("selecting 'loudnorm-streaming' calls onChange with loudnorm true and preset 'streaming'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, normalizeAudio: null }, onChange);
        openNormSelect();
        fireEvent.click(
            screen.getByRole("option", { name: /loudnorm.*streaming/i }),
        );
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.normalizeAudio).toBe(true);
        expect(called.loudnorm).toBe(true);
        expect(called.loudnormPreset).toBe("streaming");
    });

    it("selecting 'loudnorm-broadcast' calls onChange with loudnorm true and preset 'broadcast'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, normalizeAudio: null }, onChange);
        openNormSelect();
        fireEvent.click(
            screen.getByRole("option", { name: /loudnorm.*broadcast/i }),
        );
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.normalizeAudio).toBe(true);
        expect(called.loudnorm).toBe(true);
        expect(called.loudnormPreset).toBe("broadcast");
    });
});

// ---------------------------------------------------------------------------
// E. Quality preset radio buttons
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

    it("clicking 'Best Quality' preset calls onChange with best selector", () => {
        const onChange = vi.fn();
        renderPanel(defaultOptions, onChange);
        fireEvent.click(screen.getByRole("radio", { name: /best quality/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        const preset = PRESETS.find((p) => p.id === "best")!;
        expect(called.format).toBe(preset.selector);
    });

    it("clicking 'Audio Only' preset calls onChange with audio-only selector", () => {
        const onChange = vi.fn();
        renderPanel(defaultOptions, onChange);
        fireEvent.click(screen.getByRole("radio", { name: /audio only/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        const preset = PRESETS.find((p) => p.id === "audio-only")!;
        expect(called.format).toBe(preset.selector);
    });

    it("current format matching 1080p preset shows that preset as selected", () => {
        const preset = PRESETS.find((p) => p.id === "1080p")!;
        renderPanel({ ...defaultOptions, format: preset.selector });
        const radio = screen.getByRole("radio", { name: /1080p/i });
        expect(radio).toHaveAttribute("aria-checked", "true");
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
// F. Remux and Extract Audio selects
// ---------------------------------------------------------------------------

describe("Remux Select", () => {
    function getRemuxSelect() {
        const label = screen.getByText(/^remux$/i);
        const container = label.closest("div.flex-col")!;
        return container.querySelector('[role="combobox"]') as HTMLElement;
    }

    it("shows 'None' when remux is null", () => {
        renderPanel({ ...defaultOptions, remux: null });
        expect(getRemuxSelect()).toHaveTextContent(/^none$/i);
    });

    it.each([
        { name: "selecting MP4", option: /^mp4$/i, initial: null as null, field: "remux", expected: "mp4" },
        { name: "selecting MKV", option: /^mkv$/i, initial: null as null, field: "remux", expected: "mkv" },
        { name: "selecting None", option: /^none$/i, initial: "mp4" as string | null, field: "remux", expected: null },
    ])("$name calls onChange with remux: $expected", ({ option, initial, field, expected }) => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, [field]: initial }, onChange);
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: option }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called[field as keyof DownloadOptions]).toBe(expected);
    });
});

describe("Extract Audio Select", () => {
    function getAudioSelect() {
        const label = screen.getByText(/extract audio/i);
        const container = label.closest("div.flex-col")!;
        return container.querySelector('[role="combobox"]') as HTMLElement;
    }

    it("shows 'None' when extractAudio is null", () => {
        renderPanel({ ...defaultOptions, extractAudio: null });
        expect(getAudioSelect()).toHaveTextContent(/^none$/i);
    });

    it.each([
        { name: "selecting MP3", option: /^mp3$/i, expected: "mp3" },
        { name: "selecting Opus", option: /^opus$/i, expected: "opus" },
    ])("$name calls onChange with extractAudio: $expected", ({ option, expected }) => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, extractAudio: null }, onChange);
        fireEvent.pointerDown(getAudioSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: option }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.extractAudio).toBe(expected);
    });
});
