import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DownloadSection } from "./DownloadSection";
import { effectiveNetworkStub, networkDefaultsStub, networkRangesStub } from "@/test/effectiveNetworkStub";
import { appSettingsStub as baseDraft } from "@/test/appSettingsStub";
import { byteRangeToMib, bytesToMibDisplay } from "@/views/settings/byteUnits";
import type { AppSettings } from "@/types";

// NOTE (empirically established in Task 2 — do NOT use `role="spinbutton"` here):
// React Aria's `useNumberField` deliberately sets `role: null` and nulls
// `aria-valuenow`/`valuemin`/`valuemax`/`valuetext` on the rendered input
// (`@react-aria/numberfield/dist/useNumberField.mjs:199-206`, comment: "override the
// spinbutton role, we can't focus a spin button with VO"), substituting
// `aria-roledescription="Number field"`. The queryable role is therefore `textbox`.
describe("DownloadSection", () => {
    it("renders all four numeric controls", () => {
        render(<DownloadSection draft={baseDraft} network={networkDefaultsStub} onChange={vi.fn()} />);
        expect(screen.getByRole("textbox", { name: /concurrent fragments/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /buffer size/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /parallel threshold/i })).toBeInTheDocument();
        expect(screen.getByRole("textbox", { name: /probe timeout/i })).toBeInTheDocument();
    });

    it("displays a byte-valued setting in MiB, not bytes", () => {
        const draft = { ...baseDraft, buffer_size: 2 * 1_048_576 } as AppSettings;
        render(<DownloadSection draft={draft} network={networkDefaultsStub} onChange={vi.fn()} />);
        expect(screen.getByRole("textbox", { name: /buffer size/i })).toHaveValue("2");
    });

    it("commits a MiB edit back as bytes", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        const draft = { ...baseDraft, buffer_size: 2 * 1_048_576 } as AppSettings;
        render(<DownloadSection draft={draft} network={networkDefaultsStub} onChange={onChange} />);
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
        render(<DownloadSection draft={draft} network={networkDefaultsStub} onChange={onChange} />);
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
        render(<DownloadSection draft={draft} network={networkDefaultsStub} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /buffer size/i });
        await user.click(input);
        await user.tab();
        expect(onChange).not.toHaveBeenCalled();
    });

    it("renders the true byte count for a sub-MiB value instead of a misleading 0", () => {
        const draft = { ...baseDraft, buffer_size: 500_000 } as AppSettings;
        render(<DownloadSection draft={draft} network={networkDefaultsStub} onChange={vi.fn()} />);
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
        render(<DownloadSection draft={draft} network={networkDefaultsStub} onChange={onChange} />);
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
        render(<DownloadSection draft={baseDraft} network={networkDefaultsStub} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /concurrent fragments/i });
        await user.type(input, "16");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ concurrent_fragments: 16 });
    });

    // #611: every placeholder is the "inherit" hint and must be derived from the
    // IPC-sourced `EffectiveNetwork` payload, never a literal. Byte-valued fields
    // show the payload's bytes projected to whole MiB, matching the field's unit.
    it("every numeric placeholder is derived from the effective-network payload", () => {
        render(<DownloadSection draft={baseDraft} network={networkDefaultsStub} onChange={vi.fn()} />);
        const stub = effectiveNetworkStub;
        expect(screen.getByRole("textbox", { name: /concurrent fragments/i })).toHaveAttribute(
            "placeholder",
            String(stub.concurrent_fragments),
        );
        expect(screen.getByRole("textbox", { name: /probe timeout/i })).toHaveAttribute(
            "placeholder",
            String(stub.hls_head_probe_timeout_secs),
        );
        expect(screen.getByRole("textbox", { name: /buffer size/i })).toHaveAttribute(
            "placeholder",
            String(bytesToMibDisplay(stub.buffer_size)),
        );
        expect(screen.getByRole("textbox", { name: /parallel threshold/i })).toHaveAttribute(
            "placeholder",
            String(bytesToMibDisplay(stub.parallel_threshold)),
        );
    });

    it("describes an empty field as inheriting from the base configuration, not a \"default\"", () => {
        render(<DownloadSection draft={baseDraft} network={networkDefaultsStub} onChange={vi.fn()} />);
        const input = screen.getByRole("textbox", { name: /concurrent fragments/i });
        const describedBy = input.getAttribute("aria-describedby") ?? "";
        const description = describedBy
            .split(/\s+/)
            .map((id) => document.getElementById(id)?.textContent ?? "")
            .join(" ");
        expect(description).toMatch(/inherit/i);
        expect(description).toMatch(/base configuration/i);
        expect(description).not.toMatch(/default/i);
    });
});

// #611 review: every `minValue`/`maxValue` comes from the IPC-sourced `ranges`
// (the byte-valued ones projected to whole MiB), never a copied literal. Under
// NumericField's clamping contract an out-of-range entry commits as the bound,
// which is how the bounds are observable. The stub's bounds differ from the
// real ones, so a control clamping to a literal fails.
describe("DownloadSection — bounds derive from the network payload", () => {
    it("clamps the count and timeout controls to the payload's ranges", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(<DownloadSection draft={baseDraft} network={networkDefaultsStub} onChange={onChange} />);
        const r = networkRangesStub;
        const cases: [RegExp, keyof AppSettings, { min: number; max: number }][] = [
            [/concurrent fragments/i, "concurrent_fragments", r.concurrent_fragments],
            [/probe timeout/i, "hls_head_probe_timeout", r.hls_head_probe_timeout_secs],
        ];
        for (const [label, field, range] of cases) {
            const input = screen.getByRole("textbox", { name: label });
            await user.clear(input);
            await user.type(input, "0");
            await user.tab();
            expect(onChange).toHaveBeenLastCalledWith({ [field]: range.min });
            await user.clear(input);
            await user.type(input, "99999999");
            await user.tab();
            expect(onChange).toHaveBeenLastCalledWith({ [field]: range.max });
        }
    });

    it("clamps the byte-valued controls to the payload's ranges in whole MiB", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(<DownloadSection draft={baseDraft} network={networkDefaultsStub} onChange={onChange} />);
        const cases: [RegExp, keyof AppSettings, { min: number; max: number }][] = [
            [/buffer size/i, "buffer_size", byteRangeToMib(networkRangesStub.buffer_size)],
            [/parallel threshold/i, "parallel_threshold", byteRangeToMib(networkRangesStub.parallel_threshold)],
        ];
        for (const [label, field, mib] of cases) {
            const input = screen.getByRole("textbox", { name: label });
            await user.clear(input);
            await user.type(input, "99999999");
            await user.tab();
            expect(onChange).toHaveBeenLastCalledWith({ [field]: mib.max * 1_048_576 });
            // The whole-MiB floor is 1 MiB: 1 byte cannot be expressed in MiB.
            expect(mib.min).toBe(1);
        }
    });
});
