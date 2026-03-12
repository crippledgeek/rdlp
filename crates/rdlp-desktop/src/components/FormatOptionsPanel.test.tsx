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

/** Get the combobox for Audio Normalization. */
function getNormSelect() {
    const label = screen.getByText(/audio normalization/i);
    const container = label.closest("div.flex-col")!;
    return container.querySelector('[role="combobox"]') as HTMLElement;
}

function getRemuxSelect() {
    const label = screen.getByText(/^remux$/i);
    const container = label.closest("div.flex-col")!;
    return container.querySelector('[role="combobox"]') as HTMLElement;
}

function getAudioSelect() {
    const label = screen.getByText(/extract audio/i);
    const container = label.closest("div.flex-col")!;
    return container.querySelector('[role="combobox"]') as HTMLElement;
}

function openNormSelect() {
    fireEvent.pointerDown(getNormSelect(), { button: 0, pointerType: "mouse" });
}

// ---------------------------------------------------------------------------
// A. Default state — single render, multiple assertions
// ---------------------------------------------------------------------------

describe("default state (single render)", () => {
    it("renders correct initial state for all controls", () => {
        const { container } = renderPanel(defaultOptions);

        // No preset radio should be checked (format: null)
        const radios = container.querySelectorAll('[role="radio"]');
        for (const r of radios) {
            expect(r).toHaveAttribute("aria-checked", "false");
        }

        // Normalization shows "Use Settings Default"
        expect(getNormSelect()).toHaveTextContent(/use settings default/i);

        // Remux shows "None"
        expect(getRemuxSelect()).toHaveTextContent(/^none$/i);

        // Extract Audio shows "None"
        expect(getAudioSelect()).toHaveTextContent(/^none$/i);
    });
});

// ---------------------------------------------------------------------------
// B. Preset selection display
// ---------------------------------------------------------------------------

describe("activePreset display", () => {
    it("format matching a preset selector selects that preset", () => {
        const bestPreset = PRESETS.find((p) => p.id === "best")!;
        renderPanel({ ...defaultOptions, format: bestPreset.selector });
        expect(screen.getByRole("radio", { name: /best quality/i }))
            .toHaveAttribute("aria-checked", "true");
    });

    it("format matching 720p/1080p presets selects correctly", () => {
        const preset = PRESETS.find((p) => p.id === "720p")!;
        renderPanel({ ...defaultOptions, format: preset.selector });
        expect(screen.getByRole("radio", { name: /720p/i }))
            .toHaveAttribute("aria-checked", "true");
    });

    it("unrecognized format selector does not select any preset", () => {
        const { container } = renderPanel({ ...defaultOptions, format: "custom-selector" });
        const radios = container.querySelectorAll('[role="radio"]');
        for (const r of radios) {
            expect(r).toHaveAttribute("aria-checked", "false");
        }
    });
});

// ---------------------------------------------------------------------------
// C. Preset click handlers — single render, multiple clicks
// ---------------------------------------------------------------------------

describe("Quality preset radio buttons", () => {
    it("clicking presets calls onChange with the correct selector string", () => {
        const onChange = vi.fn();
        renderPanel(defaultOptions, onChange);

        // 720p
        fireEvent.click(screen.getByRole("radio", { name: /720p/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        expect((onChange.mock.calls[0][0] as DownloadOptions).format)
            .toBe(PRESETS.find((p) => p.id === "720p")!.selector);

        // Best Quality
        fireEvent.click(screen.getByRole("radio", { name: /best quality/i }));
        expect(onChange).toHaveBeenCalledTimes(2);
        expect((onChange.mock.calls[1][0] as DownloadOptions).format)
            .toBe(PRESETS.find((p) => p.id === "best")!.selector);

        // Audio Only
        fireEvent.click(screen.getByRole("radio", { name: /audio only/i }));
        expect(onChange).toHaveBeenCalledTimes(3);
        expect((onChange.mock.calls[2][0] as DownloadOptions).format)
            .toBe(PRESETS.find((p) => p.id === "audio-only")!.selector);
    });

    it("current format matching 1080p preset shows that preset as selected", () => {
        const preset = PRESETS.find((p) => p.id === "1080p")!;
        renderPanel({ ...defaultOptions, format: preset.selector });
        expect(screen.getByRole("radio", { name: /1080p/i }))
            .toHaveAttribute("aria-checked", "true");
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
// D. Audio Normalization Select — displayed value
// ---------------------------------------------------------------------------

describe("Audio Normalization Select — displayed value", () => {
    it("normalizeAudio false shows 'Off'", () => {
        renderPanel({ ...defaultOptions, normalizeAudio: false, loudnorm: false });
        expect(getNormSelect()).toHaveTextContent(/^off$/i);
    });

    it("normalizeAudio true, loudnorm false shows 'Peak'", () => {
        renderPanel({ ...defaultOptions, normalizeAudio: true, loudnorm: false });
        expect(getNormSelect()).toHaveTextContent(/^peak$/i);
    });

    it.each([
        ["streaming", "streaming", /loudnorm.*streaming/i],
        ["broadcast", "broadcast", /loudnorm.*broadcast/i],
        ["loud", "loud", /loudnorm.*loud/i],
        ["null (defaults to streaming)", null, /loudnorm.*streaming/i],
    ] as const)("loudnorm preset %s shows correct label", (_label, preset, pattern) => {
        renderPanel({
            ...defaultOptions,
            normalizeAudio: true,
            loudnorm: true,
            loudnormPreset: preset as string | null,
        });
        expect(getNormSelect()).toHaveTextContent(pattern);
    });
});

// ---------------------------------------------------------------------------
// E. Audio Normalization Select — onChange branches
// ---------------------------------------------------------------------------

describe("Audio Normalization Select — onChange branches", () => {
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

    it("selecting loudnorm presets calls onChange with correct preset value", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, normalizeAudio: null }, onChange);
        openNormSelect();
        fireEvent.click(screen.getByRole("option", { name: /loudnorm.*streaming/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.normalizeAudio).toBe(true);
        expect(called.loudnorm).toBe(true);
        expect(called.loudnormPreset).toBe("streaming");
    });

    it("selecting 'loudnorm-broadcast' calls onChange with preset 'broadcast'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, normalizeAudio: null }, onChange);
        openNormSelect();
        fireEvent.click(screen.getByRole("option", { name: /loudnorm.*broadcast/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        const called = onChange.mock.calls[0][0] as DownloadOptions;
        expect(called.loudnormPreset).toBe("broadcast");
    });
});

// ---------------------------------------------------------------------------
// F. Remux Select
// ---------------------------------------------------------------------------

describe("Remux Select", () => {
    it("selecting options calls onChange with correct remux value", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, remux: null }, onChange);

        // Select MP4
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mp4$/i }));
        expect(onChange).toHaveBeenCalledTimes(1);
        expect((onChange.mock.calls[0][0] as DownloadOptions).remux).toBe("mp4");
    });

    it("selecting MKV calls onChange with remux: 'mkv'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, remux: null }, onChange);
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mkv$/i }));
        expect((onChange.mock.calls[0][0] as DownloadOptions).remux).toBe("mkv");
    });

    it("selecting None calls onChange with remux: null", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, remux: "mp4" }, onChange);
        fireEvent.pointerDown(getRemuxSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^none$/i }));
        expect((onChange.mock.calls[0][0] as DownloadOptions).remux).toBeNull();
    });
});

// ---------------------------------------------------------------------------
// G. Extract Audio Select
// ---------------------------------------------------------------------------

describe("Extract Audio Select", () => {
    it("selecting MP3 calls onChange with extractAudio: 'mp3'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, extractAudio: null }, onChange);
        fireEvent.pointerDown(getAudioSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^mp3$/i }));
        expect((onChange.mock.calls[0][0] as DownloadOptions).extractAudio).toBe("mp3");
    });

    it("selecting Opus calls onChange with extractAudio: 'opus'", () => {
        const onChange = vi.fn();
        renderPanel({ ...defaultOptions, extractAudio: null }, onChange);
        fireEvent.pointerDown(getAudioSelect(), { button: 0, pointerType: "mouse" });
        fireEvent.click(screen.getByRole("option", { name: /^opus$/i }));
        expect((onChange.mock.calls[0][0] as DownloadOptions).extractAudio).toBe("opus");
    });
});
