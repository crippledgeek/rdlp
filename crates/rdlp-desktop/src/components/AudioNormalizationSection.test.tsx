// Tests for AudioNormalizationSection — isolated from full SettingsPage render.

import { render, screen, fireEvent } from "@/test/test-utils";
import { AudioNormalizationSection } from "./AudioNormalizationSection";
import type { AppSettings } from "../types";

const baseSettings: AppSettings = {
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

function renderSection(draft: AppSettings, onChange = vi.fn()) {
    return render(
        <AudioNormalizationSection draft={draft} onChange={onChange} />,
    );
}

describe("AudioNormalizationSection", () => {
    it("shows normalize audio checkbox unchecked by default", () => {
        renderSection(baseSettings);
        const label = screen.getByText(/normalize audio/i);
        const checkbox = label.closest("div")?.querySelector('button[role="checkbox"]');
        expect(checkbox).toHaveAttribute("aria-checked", "false");
    });

    it("calls onChange when normalize audio is checked", () => {
        const onChange = vi.fn();
        renderSection(baseSettings, onChange);
        const label = screen.getByText(/normalize audio/i);
        const checkbox = label.closest("div")?.querySelector('button[role="checkbox"]') as HTMLElement;
        fireEvent.click(checkbox);
        expect(onChange).toHaveBeenCalledWith({ normalize_audio: true });
    });

    it("reveals mode toggle when normalize_audio is true", () => {
        renderSection({ ...baseSettings, normalize_audio: true });
        expect(screen.getByText(/ebu r128 loudnorm/i)).toBeInTheDocument();
    });

    it("reveals boost fallback when normalize_audio is true", () => {
        renderSection({ ...baseSettings, normalize_audio: true });
        expect(screen.getByText(/boost fallback/i)).toBeInTheDocument();
    });

    it("shows preset select and advanced options when loudnorm mode is active", () => {
        renderSection({ ...baseSettings, normalize_audio: true, loudnorm: true });
        expect(screen.getByText(/dynamic mode/i)).toBeInTheDocument();
        expect(screen.getByText(/precompress/i)).toBeInTheDocument();
        expect(screen.getByText("Preset")).toBeInTheDocument();
    });

    it("shows boost gain input when boost fallback is checked", () => {
        renderSection({ ...baseSettings, normalize_audio: true, normalize_boost: true });
        expect(screen.getByPlaceholderText("12.0")).toBeInTheDocument();
    });

    it("hides mode toggle when normalize_audio is unchecked", () => {
        renderSection(baseSettings);
        expect(screen.queryByText(/ebu r128 loudnorm/i)).not.toBeInTheDocument();
    });

    it("hides preset select when mode is Peak", () => {
        renderSection({ ...baseSettings, normalize_audio: true, loudnorm: false });
        expect(screen.queryByText(/dynamic mode/i)).not.toBeInTheDocument();
        expect(screen.queryByText(/precompress/i)).not.toBeInTheDocument();
        expect(screen.queryByText("Preset")).not.toBeInTheDocument();
    });

    it("hides boost dB input when boost fallback is unchecked", () => {
        renderSection({ ...baseSettings, normalize_audio: true, normalize_boost: false });
        expect(screen.queryByPlaceholderText("12.0")).not.toBeInTheDocument();
    });
});
