import { describe, it, expect, vi } from "vitest";
import { fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
    download_timeout: null,
    merge_timeout: null,
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
    write_subtitles: false,
    write_auto_subtitles: false,
    strict_subs: false,
    verify_sub_urls: false,
    retry_subs: false,
    concurrent_fragments: null,
    buffer_size: null,
    parallel_threshold: null,
    hls_head_probe_timeout: null,
};

// NOTE on role: NumericField wraps React Aria's NumberField, which
// deliberately overrides the ARIA APG spinbutton role to `null` on the
// rendered <input> (VoiceOver focus incompatibility) — the input exposes the
// implicit `textbox` role instead. See NumericField.test.tsx / task-2-report.md
// for the full empirical finding. Queries below use role="textbox".
//
// NOTE on commit timing: NumericField (React Aria useNumberFieldState) commits
// on blur, not on every keystroke — unlike the hand-rolled TimeoutField this
// section used to render. Tests that assert `onChange` therefore drive input
// via `userEvent` (type + tab) rather than a single `fireEvent.change`.
describe("NetworkSection — timeout controls", () => {
    it("renders four timeout controls with associated labels", () => {
        render(<NetworkSection draft={baseDraft} onChange={vi.fn()} />);
        expect(screen.getByRole("textbox", { name: /connection timeout/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /read timeout/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /download timeout/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /merge timeout/i })).toBeInTheDocument();
        expect(
            screen.getByRole("checkbox", { name: /evict idle/i }),
        ).toBeInTheDocument();
    });

    it("typing in connection timeout commits a number on blur", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /connection timeout/i });
        await user.clear(input);
        await user.type(input, "45");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: 45 });
    });

    it("emptying connection timeout commits null", async () => {
        const user = userEvent.setup();
        const draft = { ...baseDraft, socket_timeout: 30 };
        const onChange = vi.fn();
        render(<NetworkSection draft={draft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /connection timeout/i });
        await user.clear(input);
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: null });
    });

    // DESIGNED BEHAVIOUR: NumericField enforces bounds by CLAMPING, not
    // rejection (React Aria's useNumberFieldState.commit() clamps before any
    // validation runs — see NumericField.tsx's doc comment). An out-of-range
    // connection timeout is silently coerced to maxValue=300, never rejected.
    it("out-of-range connection timeout clamps to the upper bound", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /connection timeout/i });
        await user.clear(input);
        await user.type(input, "9999");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: 300 });
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
        const numeric = screen.getByRole("textbox", { name: /idle timeout/i });
        expect(numeric).toBeDisabled();
    });

    it("updates connection-timeout display when draft prop changes externally", () => {
        const onChange = vi.fn();
        const { rerender } = render(
            <NetworkSection draft={{ ...baseDraft, socket_timeout: 30 }} onChange={onChange} />,
        );
        let input = screen.getByRole("textbox", { name: /connection timeout/i });
        expect(input).toHaveValue("30");
        rerender(<NetworkSection draft={{ ...baseDraft, socket_timeout: 60 }} onChange={onChange} />);
        input = screen.getByRole("textbox", { name: /connection timeout/i });
        expect(input).toHaveValue("60");
    });

    // Finding 1 regression guard: the pool-idle NumericField renders `helper=""`
    // (its helper text lives in a sibling FormDescription for layout reasons) and
    // must stay programmatically associated with that sibling via
    // aria-describedby, rather than losing its accessible description entirely.
    it("associates the idle-timeout field with its sibling helper text via aria-describedby", () => {
        render(<NetworkSection draft={baseDraft} onChange={vi.fn()} />);
        const input = screen.getByRole("textbox", { name: /idle timeout/i });
        const describedBy = input.getAttribute("aria-describedby");
        expect(describedBy).toBeTruthy();
        const describedByIds = describedBy!.split(/\s+/);
        expect(describedByIds).toContain("pool-idle-timeout-description");
        const descriptionNode = document.getElementById("pool-idle-timeout-description");
        expect(descriptionNode).not.toBeNull();
        expect(descriptionNode!.textContent).toMatch(/idle keep-alive connections/i);
    });

    // #613 regression guard: the placeholder is the "inherit the backend default"
    // hint, so it must state the value the app actually uses —
    // `DEFAULT_POOL_IDLE_TIMEOUT_SECS` in rdlp-http/src/config.rs, which owns the
    // rationale for that number. Pinning a literal against a literal is weak by
    // construction; #611 replaces it by sourcing the value over IPC.
    it("pool-idle placeholder states the real backend default (60, not 90)", () => {
        render(<NetworkSection draft={baseDraft} onChange={vi.fn()} />);
        const numeric = screen.getByRole("textbox", { name: /idle timeout/i });
        expect(numeric).toHaveAttribute("placeholder", "60");
    });

    it("out-of-range pool-idle value clamps to the upper bound", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(
            <NetworkSection draft={{ ...baseDraft, pool_idle_timeout: 90 }} onChange={onChange} />,
        );
        const numeric = screen.getByRole("textbox", { name: /idle timeout/i });
        await user.clear(numeric);
        await user.type(numeric, "9999");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ pool_idle_timeout: 3600 });
    });
});
