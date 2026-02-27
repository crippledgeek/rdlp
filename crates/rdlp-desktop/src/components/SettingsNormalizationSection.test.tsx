import { render, screen } from "@/test/test-utils";
import userEvent from "@testing-library/user-event";
import { SettingsNormalizationSection } from "./SettingsNormalizationSection";
import type { AppSettings } from "../types";

const baseSettings: AppSettings = {
    output_dir: "/tmp",
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

describe("SettingsNormalizationSection", () => {
    it("renders the Audio Normalization heading", () => {
        render(
            <SettingsNormalizationSection draft={baseSettings} onChange={vi.fn()} />,
        );
        expect(screen.getByText("Audio Normalization")).toBeInTheDocument();
    });

    it("renders normalize audio checkbox unchecked by default", () => {
        render(
            <SettingsNormalizationSection draft={baseSettings} onChange={vi.fn()} />,
        );
        const label = screen.getByText(/normalize audio/i);
        const checkbox = label.closest("div")?.querySelector('button[role="checkbox"]');
        expect(checkbox).toHaveAttribute("aria-checked", "false");
    });

    it("calls onChange when normalize audio checkbox is toggled", async () => {
        const onChange = vi.fn();
        render(
            <SettingsNormalizationSection draft={baseSettings} onChange={onChange} />,
        );
        const label = screen.getByText(/normalize audio/i);
        const checkbox = label.closest("div")?.querySelector('button[role="checkbox"]');
        await userEvent.click(checkbox!);
        expect(onChange).toHaveBeenCalledWith(
            expect.objectContaining({ normalize_audio: true }),
        );
    });

    it("does not show mode toggle when normalize_audio is false", () => {
        render(
            <SettingsNormalizationSection draft={baseSettings} onChange={vi.fn()} />,
        );
        expect(screen.queryByText(/ebu r128 loudnorm/i)).not.toBeInTheDocument();
    });

    it("shows mode toggle and boost fallback when normalize_audio is true", () => {
        const draft = { ...baseSettings, normalize_audio: true };
        render(
            <SettingsNormalizationSection draft={draft} onChange={vi.fn()} />,
        );
        expect(screen.getByText(/ebu r128 loudnorm/i)).toBeInTheDocument();
        expect(screen.getByText(/boost fallback/i)).toBeInTheDocument();
    });

    it("shows peak target input when mode is Peak", () => {
        const draft = { ...baseSettings, normalize_audio: true, loudnorm: false };
        render(
            <SettingsNormalizationSection draft={draft} onChange={vi.fn()} />,
        );
        expect(screen.getByLabelText(/peak target/i)).toBeInTheDocument();
    });

    it("hides peak target and shows loudnorm controls when mode is Loudnorm", () => {
        const draft = { ...baseSettings, normalize_audio: true, loudnorm: true };
        render(
            <SettingsNormalizationSection draft={draft} onChange={vi.fn()} />,
        );
        expect(screen.queryByLabelText(/peak target/i)).not.toBeInTheDocument();
        expect(screen.getByText("Preset")).toBeInTheDocument();
        expect(screen.getByText(/dynamic mode/i)).toBeInTheDocument();
        expect(screen.getByText(/precompress/i)).toBeInTheDocument();
    });

    it("shows loudnorm custom target inputs", () => {
        const draft = { ...baseSettings, normalize_audio: true, loudnorm: true };
        render(
            <SettingsNormalizationSection draft={draft} onChange={vi.fn()} />,
        );
        expect(screen.getByLabelText(/loudness target/i)).toBeInTheDocument();
        expect(screen.getByLabelText(/true peak limit/i)).toBeInTheDocument();
        expect(screen.getByLabelText(/loudness range/i)).toBeInTheDocument();
    });

    it("shows boost gain input when boost fallback is checked", () => {
        const draft = { ...baseSettings, normalize_audio: true, normalize_boost: true };
        render(
            <SettingsNormalizationSection draft={draft} onChange={vi.fn()} />,
        );
        expect(screen.getByPlaceholderText("12.0")).toBeInTheDocument();
    });

    it("hides boost gain input when boost fallback is unchecked", () => {
        const draft = { ...baseSettings, normalize_audio: true, normalize_boost: false };
        render(
            <SettingsNormalizationSection draft={draft} onChange={vi.fn()} />,
        );
        expect(screen.queryByPlaceholderText("12.0")).not.toBeInTheDocument();
    });
});
