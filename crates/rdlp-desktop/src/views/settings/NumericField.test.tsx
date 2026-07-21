import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NumericField } from "./NumericField";

function setup(overrides: Partial<React.ComponentProps<typeof NumericField>> = {}) {
    const onCommit = vi.fn();
    render(
        <NumericField
            id="test-field"
            label="Test Field"
            helper="A helper line."
            value={8}
            minValue={1}
            maxValue={64}
            onCommit={onCommit}
            {...overrides}
        />,
    );
    return { onCommit };
}

// NOTE on role: React Aria's `useNumberField` deliberately overrides the ARIA
// APG spinbutton role to `null` on the rendered <input> — see
// @react-aria/numberfield's useNumberField.js: "override the spinbutton role,
// we can't focus a spin button with VO" (VoiceOver). The input therefore
// exposes the implicit `textbox` role plus `aria-roledescription="Number
// field"`, and `aria-valuenow`/`aria-valuemin`/`aria-valuemax` are explicitly
// nulled out — never rendered on the DOM in this React Aria version. This was
// verified against the installed node_modules source, not assumed. The first
// test below was corrected from the brief's `role="spinbutton"` premise to
// match; see task-2-report.md for the full empirical finding.
describe("NumericField", () => {
    it("exposes an accessible textbox carrying number-field ARIA semantics", () => {
        setup();
        const input = screen.getByRole("textbox", { name: /test field/i });
        // Assert presence, not the exact string: "Number field" is React Aria's
        // default-LOCALE string, not something this component controls — pinning
        // the value would make the test locale-fragile for a reason unrelated to
        // what it means to prove (that number-field ARIA semantics were applied).
        expect(input).toHaveAttribute("aria-roledescription");
        expect(input).toHaveValue("8");
    });

    it("derives its accessible name from the visible label only (no aria-label override)", () => {
        setup();
        const input = screen.getByRole("textbox", { name: /test field/i });
        expect(input).not.toHaveAttribute("aria-label");
    });

    it("renders the helper text", () => {
        setup();
        expect(screen.getByText("A helper line.")).toBeInTheDocument();
    });

    it("renders a suffix when supplied", () => {
        setup({ suffix: "MiB" });
        expect(screen.getByText("MiB")).toBeInTheDocument();
    });

    it("renders the placeholder on the input when supplied", () => {
        setup({ value: null, placeholder: "30" });
        expect(screen.getByRole("textbox", { name: /test field/i })).toHaveAttribute(
            "placeholder",
            "30",
        );
    });

    it("renders an empty input when value is null", () => {
        setup({ value: null });
        expect(screen.getByRole("textbox", { name: /test field/i })).toHaveValue("");
    });

    it("commits a parsed number on blur", async () => {
        const user = userEvent.setup();
        const { onCommit } = setup();
        const input = screen.getByRole("textbox", { name: /test field/i });
        await user.clear(input);
        await user.type(input, "16");
        await user.tab();
        expect(onCommit).toHaveBeenCalledWith(16);
    });

    it("commits null when the field is cleared", async () => {
        const user = userEvent.setup();
        const { onCommit } = setup();
        const input = screen.getByRole("textbox", { name: /test field/i });
        await user.clear(input);
        await user.tab();
        expect(onCommit).toHaveBeenCalledWith(null);
    });

    it("does not commit while the user is still typing", async () => {
        const user = userEvent.setup();
        const { onCommit } = setup();
        const input = screen.getByRole("textbox", { name: /test field/i });
        await user.clear(input);
        await user.type(input, "1");
        expect(onCommit).not.toHaveBeenCalledWith(1);
    });

    // DESIGNED BEHAVIOUR: bounds are enforced by CLAMPING, not rejection.
    // React Aria's useNumberFieldState unconditionally clamps the committed
    // value to [minValue, maxValue] *inside* `commit()`, before validation
    // ever runs against it — see useNumberFieldState.module.js:
    // `clamp(newParsedValue, minValue, maxValue)` precedes
    // `validation.commitValidation()`. This is the component's ONLY range
    // mechanism (there is no schema prop): an out-of-range commit is
    // silently coerced to the nearest bound, `onCommit` fires with the
    // clamped value, and `FieldError`/`aria-invalid` never trigger from a
    // range violation.
    it("clamps an out-of-range commit to the nearest bound", async () => {
        const user = userEvent.setup();
        const { onCommit } = setup();
        const input = screen.getByRole("textbox", { name: /test field/i });
        await user.clear(input);
        await user.type(input, "999");
        await user.tab();
        expect(onCommit).toHaveBeenCalledWith(64);
        expect(
            screen.queryByText("Number must be less than or equal to 64"),
        ).not.toBeInTheDocument();
        expect(input).not.toHaveAttribute("aria-invalid");
    });
});
