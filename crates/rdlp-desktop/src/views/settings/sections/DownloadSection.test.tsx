import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DownloadSection } from "./DownloadSection";
import type { AppSettings } from "@/types";

const baseDraft = {
    concurrent_fragments: null,
    buffer_size: null,
    parallel_threshold: null,
    hls_head_probe_timeout: null,
} as unknown as AppSettings;

// NOTE (empirically established in Task 2 — do NOT use `role="spinbutton"` here):
// React Aria's `useNumberField` deliberately sets `role: null` and nulls
// `aria-valuenow`/`valuemin`/`valuemax`/`valuetext` on the rendered input
// (`@react-aria/numberfield/dist/useNumberField.mjs:199-206`, comment: "override the
// spinbutton role, we can't focus a spin button with VO"), substituting
// `aria-roledescription="Number field"`. The queryable role is therefore `textbox`.
describe("DownloadSection", () => {
    it("renders all four numeric controls", () => {
        render(<DownloadSection draft={baseDraft} onChange={vi.fn()} />);
        expect(screen.getByRole("textbox", { name: /concurrent fragments/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /buffer size/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /parallel threshold/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /probe timeout/i })).toBeInTheDocument();
    });

    it("displays a byte-valued setting in MiB, not bytes", () => {
        const draft = { ...baseDraft, buffer_size: 2 * 1_048_576 } as AppSettings;
        render(<DownloadSection draft={draft} onChange={vi.fn()} />);
        expect(screen.getByRole("textbox", { name: /buffer size/i })).toHaveValue("2");
    });

    it("commits a MiB edit back as bytes", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        const draft = { ...baseDraft, buffer_size: 2 * 1_048_576 } as AppSettings;
        render(<DownloadSection draft={draft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /buffer size/i });
        await user.clear(input);
        await user.type(input, "8");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ buffer_size: 8 * 1_048_576 });
    });

    it("commits null when a byte field is cleared, preserving inherit semantics", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        const draft = { ...baseDraft, buffer_size: 2 * 1_048_576 } as AppSettings;
        render(<DownloadSection draft={draft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /buffer size/i });
        await user.clear(input);
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ buffer_size: null });
    });

    // Contract pin, not a regression guard: a no-op focus/blur (no typing) must
    // never emit a commit. This does NOT discriminate against the originally
    // suspected mechanism — React Aria clamps a controlled `value` at
    // construction (`@react-stately/numberfield/dist/useNumberFieldState.mjs:24-25`)
    // and `useControlledState` only fires `onChange` on an actual change, so this
    // test would have passed against the unpatched code too. The genuinely
    // discriminating regression guard for the sub-MiB display bug is the
    // neighbouring test asserting the field renders empty with the true byte
    // count in the placeholder. Kept here because the "no-op means no commit"
    // contract is worth pinning on its own merits.
    it("does not rewrite a sub-MiB stored value on a no-op focus/blur", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        const draft = { ...baseDraft, buffer_size: 500_000 } as AppSettings;
        render(<DownloadSection draft={draft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /buffer size/i });
        await user.click(input);
        await user.tab();
        expect(onChange).not.toHaveBeenCalled();
    });

    it("renders the true byte count for a sub-MiB value instead of a misleading 0", () => {
        const draft = { ...baseDraft, buffer_size: 500_000 } as AppSettings;
        render(<DownloadSection draft={draft} onChange={vi.fn()} />);
        const input = screen.getByRole("textbox", { name: /buffer size/i });
        expect(input).toHaveValue("");
        expect(input).toHaveAttribute("placeholder", "500,000 B");
    });

    // Finding 2 regression guard (MiB path): verified in
    // `node_modules/@internationalized/number/dist/NumberParser.mjs:154` — with
    // `maximumFractionDigits: 0` the "." keystroke is rejected as invalid partial
    // input, not rounded. Typing "3.5" therefore never reaches `mibDisplayToBytes`
    // with 3.5 MiB; the "." is dropped and the digits concatenate, leaving "35"
    // MiB. Pin the exact byte value so the digit-concatenation mechanism is
    // visible to the next reader, not just "divisible by 1 MiB" (which 3.5 MiB
    // rounded to a whole MiB would also satisfy).
    it("commits the digit-concatenated whole-MiB byte value when a fractional MiB is typed", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        const draft = { ...baseDraft, buffer_size: 2 * 1_048_576 } as AppSettings;
        render(<DownloadSection draft={draft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /buffer size/i });
        await user.clear(input);
        await user.type(input, "3.5");
        await user.tab();
        expect(onChange).toHaveBeenCalledTimes(1);
        expect(onChange).toHaveBeenCalledWith({ buffer_size: 35 * 1_048_576 });
    });

    it("passes a unitless count straight through without conversion", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(<DownloadSection draft={baseDraft} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /concurrent fragments/i });
        await user.type(input, "16");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ concurrent_fragments: 16 });
    });
});
