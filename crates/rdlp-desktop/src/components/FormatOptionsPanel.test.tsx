import { render, screen, fireEvent } from "@/test/test-utils";
import {
    FormatOptionsPanel,
    PRESETS,
} from "./FormatOptionsPanel";
import type { DownloadOptions } from "../types";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
// A. activePreset — tested indirectly via rendered RadioGroup selection
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
        // The Audio Normalization label is immediately above its combobox.
        // There are multiple comboboxes (Remux, Extract Audio, Audio Normalization).
        // We rely on the label text to identify the right trigger.
        const label = screen.getByText(/audio normalization/i);
        // The trigger is the next sibling combobox; use closest parent container.
        const container = label.closest("div.flex-col")!;
        return container.querySelector('[role="combobox"]') as HTMLElement;
    }

    it("normalizeAudio null shows 'Use Settings Default'", () => {
        renderPanel({ ...defaultOptions, normalizeAudio: null });
        expect(getNormSelect()).toHaveTextContent(/use settings default/i);
    });

    it("normalizeAudio false shows 'Off'", () => {
        renderPanel({ ...defaultOptions, normalizeAudio: false, loudnorm: false });
        expect(getNormSelect()).toHaveTextContent(/^off$/i);
    });

    it("normalizeAudio true, loudnorm false shows 'Peak'", () => {
        renderPanel({
            ...defaultOptions,
            normalizeAudio: true,
            loudnorm: false,
        });
        expect(getNormSelect()).toHaveTextContent(/^peak$/i);
    });

    it("normalizeAudio true, loudnorm true, preset 'streaming' shows loudnorm streaming", () => {
        renderPanel({
            ...defaultOptions,
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "streaming",
        });
        expect(getNormSelect()).toHaveTextContent(/loudnorm.*streaming/i);
    });

    it("normalizeAudio true, loudnorm true, preset 'broadcast' shows loudnorm broadcast", () => {
        renderPanel({
            ...defaultOptions,
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "broadcast",
        });
        expect(getNormSelect()).toHaveTextContent(/loudnorm.*broadcast/i);
    });

    it("normalizeAudio true, loudnorm true, preset 'loud' shows loudnorm loud", () => {
        renderPanel({
            ...defaultOptions,
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: "loud",
        });
        expect(getNormSelect()).toHaveTextContent(/loudnorm.*loud/i);
    });

    it("normalizeAudio true, loudnorm true, preset null defaults to loudnorm streaming (bugfix case)", () => {
        renderPanel({
            ...defaultOptions,
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: null,
        });
        expect(getNormSelect()).toHaveTextContent(/loudnorm.*streaming/i);
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

    it("selecting MP4 calls onChange with remux: 'mp4'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, remux: null }, onChange);
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mp4$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.remux).toBe("mp4");
    });

    it("selecting MKV calls onChange with remux: 'mkv'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, remux: null }, onChange);
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mkv$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.remux).toBe("mkv");
    });

    it("selecting None calls onChange with remux: null", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, remux: "mp4" }, onChange);
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^none$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.remux).toBeNull();
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

    it("selecting MP3 calls onChange with extractAudio: 'mp3'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, extractAudio: null }, onChange);
        fireEvent.pointerDown(getAudioSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mp3$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.extractAudio).toBe("mp3");
    });

    it("selecting Opus calls onChange with extractAudio: 'opus'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, extractAudio: null }, onChange);
        fireEvent.pointerDown(getAudioSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^opus$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.extractAudio).toBe("opus");
    });
});
