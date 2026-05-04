import { describe, it, expect, vi } from "vitest";
import { fireEvent } from "@testing-library/react";
import { render, screen } from "@/test/test-utils";
import { NetworkSection } from "./NetworkSection";
import type { AppSettings } from "@/types";

const baseDraft: AppSettings = {
    output_dir: ".",
    default_remux: null,
    default_extract_audio: null,
    default_subtitle_format: null,
    default_subtitle_langs: [],
    embed_thumbnail: true,
    write_thumbnail: false,
    embed_metadata: false,
    verbose: false,
    default_search_provider: null,
    output_template: null,
    cookies_from_browser: null,
    cookies_file: null,
    proxy: null,
    rate_limit: null,
    socket_timeout: null,
    read_timeout: null,
    pool_idle_timeout: null,
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

describe("NetworkSection — timeout controls", () => {
    it("renders three timeout controls with associated labels", () => {
        render(<NetworkSection draft={baseDraft} onChange={vi.fn()} />);
        expect(screen.getByLabelText(/connection timeout/i)).toBeInTheDocument();
        expect(screen.getByLabelText(/read timeout/i)).toBeInTheDocument();
        expect(
            screen.getByRole("checkbox", { name: /evict idle/i }),
        ).toBeInTheDocument();
    });

    it("typing in connection timeout commits a number when valid", () => {
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} onChange={onChange} />);
        const input = screen.getByLabelText(
            /connection timeout/i,
        ) as HTMLInputElement;
        fireEvent.change(input, { target: { value: "45" } });
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: 45 });
    });

    it("emptying connection timeout commits null", () => {
        const draft = { ...baseDraft, socket_timeout: 30 };
        const onChange = vi.fn();
        render(<NetworkSection draft={draft} onChange={onChange} />);
        const input = screen.getByLabelText(
            /connection timeout/i,
        ) as HTMLInputElement;
        fireEvent.change(input, { target: { value: "" } });
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: null });
    });

    it("invalid connection timeout does not commit but shows inline error", () => {
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} onChange={onChange} />);
        const input = screen.getByLabelText(
            /connection timeout/i,
        ) as HTMLInputElement;
        fireEvent.change(input, { target: { value: "9999" } });
        expect(screen.getByText(/≤ 300/i)).toBeInTheDocument();
        expect(
            onChange.mock.calls.every(([arg]) => arg.socket_timeout !== 9999),
        ).toBe(true);
    });

    it("checkbox unchecked commits pool_idle_timeout=0 (sentinel)", () => {
        const draft = { ...baseDraft, pool_idle_timeout: 90 };
        const onChange = vi.fn();
        render(<NetworkSection draft={draft} onChange={onChange} />);
        const checkbox = screen.getByRole("checkbox", { name: /evict idle/i });
        fireEvent.click(checkbox);
        expect(onChange).toHaveBeenCalledWith({ pool_idle_timeout: 0 });
    });

    it("checkbox unchecked disables the numeric input", () => {
        const draft = { ...baseDraft, pool_idle_timeout: 0 };
        render(<NetworkSection draft={draft} onChange={vi.fn()} />);
        const numeric = screen.getByPlaceholderText("90") as HTMLInputElement;
        expect(numeric).toBeDisabled();
    });
});
