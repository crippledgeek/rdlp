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

    // Regression guard (security review Finding C): `bytesToMibDisplay(500_000)` rounds
    // to 0, which is below `minValue={1}`. Rendering that 0 as the field's controlled
    // value meant a blur with NO typing still clamped 0 -> 1 and committed 1 MiB,
    // silently overwriting a legitimate sub-MiB stored value. Focusing and blurring
    // with no edit must be a no-op.
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
