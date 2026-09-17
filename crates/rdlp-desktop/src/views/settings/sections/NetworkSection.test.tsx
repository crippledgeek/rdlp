import { describe, it, expect, vi } from "vitest";
import { fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { render, screen } from "@/test/test-utils";
import { NetworkSection } from "./NetworkSection";
import { builtinNetworkStub, effectiveNetworkStub, networkDefaultsStub, networkRangesStub } from "@/test/effectiveNetworkStub";
import { appSettingsStub as baseDraft } from "@/test/appSettingsStub";


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
        render(<NetworkSection draft={baseDraft} network={networkDefaultsStub} onChange={vi.fn()} />);
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
        render(<NetworkSection draft={baseDraft} network={networkDefaultsStub} onChange={onChange} />);
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
        render(<NetworkSection draft={draft} network={networkDefaultsStub} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /connection timeout/i });
        await user.clear(input);
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: null });
    });

    // DESIGNED BEHAVIOUR: NumericField enforces bounds by CLAMPING, not
    // rejection (React Aria's useNumberFieldState.commit() clamps before any
    // validation runs — see NumericField.tsx's doc comment). An out-of-range
    // connection timeout is silently coerced to the payload's maxValue, never
    // rejected. The stub's bounds differ from the real ones, so a control
    // clamping to a literal fails here (#611 review).
    it("out-of-range connection timeout clamps to the payload's upper bound", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} network={networkDefaultsStub} onChange={onChange} />);
        const input = screen.getByRole("textbox", { name: /connection timeout/i });
        await user.clear(input);
        await user.type(input, "9999");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: networkRangesStub.socket_timeout_secs.max });
    });

    // #611 review: every `minValue`/`maxValue` comes from the IPC-sourced
    // `ranges` — the same table the engine validates against — never a copied
    // literal. Under the clamping contract a typed value below `min` commits
    // as `min`, so the lower bound is observable the same way as the upper.
    it("every timeout control's bounds are the payload's ranges", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} network={networkDefaultsStub} onChange={onChange} />);
        const r = networkRangesStub;
        const cases: [RegExp, keyof typeof baseDraft, { min: number; max: number }][] = [
            [/connection timeout/i, "socket_timeout", r.socket_timeout_secs],
            [/read timeout/i, "read_timeout", r.read_timeout_secs],
            [/download timeout/i, "download_timeout", r.download_timeout_secs],
            [/merge timeout/i, "merge_timeout", r.merge_timeout_secs],
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

    it("checkbox unchecked commits pool_idle_timeout=0 (sentinel)", () => {
        const draft = { ...baseDraft, pool_idle_timeout: 90 };
        const onChange = vi.fn();
        render(<NetworkSection draft={draft} network={networkDefaultsStub} onChange={onChange} />);
        const checkbox = screen.getByRole("checkbox", { name: /evict idle/i });
        fireEvent.click(checkbox);
        expect(onChange).toHaveBeenCalledWith({ pool_idle_timeout: 0 });
    });

    it("checkbox unchecked disables the numeric input", () => {
        const draft = { ...baseDraft, pool_idle_timeout: 0 };
        render(<NetworkSection draft={draft} network={networkDefaultsStub} onChange={vi.fn()} />);
        const numeric = screen.getByRole("textbox", { name: /idle timeout/i });
        expect(numeric).toBeDisabled();
    });

    it("updates connection-timeout display when draft prop changes externally", () => {
        const onChange = vi.fn();
        const { rerender } = render(
            <NetworkSection draft={{ ...baseDraft, socket_timeout: 30 }} network={networkDefaultsStub} onChange={onChange} />,
        );
        let input = screen.getByRole("textbox", { name: /connection timeout/i });
        expect(input).toHaveValue("30");
        rerender(<NetworkSection draft={{ ...baseDraft, socket_timeout: 60 }} network={networkDefaultsStub} onChange={onChange} />);
        input = screen.getByRole("textbox", { name: /connection timeout/i });
        expect(input).toHaveValue("60");
    });

    // Finding 1 regression guard: the pool-idle NumericField renders `helper=""`
    // (its helper text lives in a sibling FormDescription for layout reasons) and
    // must stay programmatically associated with that sibling via
    // aria-describedby, rather than losing its accessible description entirely.
    it("associates the idle-timeout field with its sibling helper text via aria-describedby", () => {
        render(<NetworkSection draft={baseDraft} network={networkDefaultsStub} onChange={vi.fn()} />);
        const input = screen.getByRole("textbox", { name: /idle timeout/i });
        const describedBy = input.getAttribute("aria-describedby");
        expect(describedBy).toBeTruthy();
        const describedByIds = describedBy!.split(/\s+/);
        expect(describedByIds).toContain("pool-idle-timeout-description");
        const descriptionNode = document.getElementById("pool-idle-timeout-description");
        expect(descriptionNode).not.toBeNull();
        expect(descriptionNode!.textContent).toMatch(/idle keep-alive connections/i);
    });

    // #613 / #611: the placeholder is the "inherit" hint, so it must state the
    // value the app actually uses. That value arrives over IPC as the
    // `EffectiveNetwork` payload (one owner: `rdlp_types::EffectiveNetwork::DEFAULT`
    // resolved through `Config::effective_network()`), so every NumericField
    // placeholder here must be derived from the `defaults` prop — never a
    // literal. The stub's values are distinct from the real defaults AND from
    // each other, so a leftover literal or a field wired to the wrong key fails.
    it("every numeric placeholder is derived from the effective-network payload", () => {
        render(<NetworkSection draft={baseDraft} network={networkDefaultsStub} onChange={vi.fn()} />);
        const stub = effectiveNetworkStub;
        expect(screen.getByRole("textbox", { name: /connection timeout/i })).toHaveAttribute(
            "placeholder",
            String(stub.socket_timeout_secs),
        );
        expect(screen.getByRole("textbox", { name: /read timeout/i })).toHaveAttribute(
            "placeholder",
            String(stub.read_timeout_secs),
        );
        expect(screen.getByRole("textbox", { name: /download timeout/i })).toHaveAttribute(
            "placeholder",
            String(stub.download_timeout_secs),
        );
        expect(screen.getByRole("textbox", { name: /merge timeout/i })).toHaveAttribute(
            "placeholder",
            String(stub.merge_timeout_secs),
        );
        expect(screen.getByRole("textbox", { name: /idle timeout/i })).toHaveAttribute(
            "placeholder",
            String(stub.pool_idle_timeout_secs),
        );
    });

    it("describes an empty field as inheriting from the base configuration, not a \"default\"", () => {
        render(<NetworkSection draft={baseDraft} network={networkDefaultsStub} onChange={vi.fn()} />);
        const input = screen.getByRole("textbox", { name: /connection timeout/i });
        const describedBy = input.getAttribute("aria-describedby") ?? "";
        const description = describedBy
            .split(/\s+/)
            .map((id) => document.getElementById(id)?.textContent ?? "")
            .join(" ");
        expect(description).toMatch(/inherit/i);
        expect(description).toMatch(/base configuration/i);
        expect(description).not.toMatch(/default/i);
    });

    it("out-of-range pool-idle value clamps to the upper bound", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(
            <NetworkSection draft={{ ...baseDraft, pool_idle_timeout: 90 }} network={networkDefaultsStub} onChange={onChange} />,
        );
        const numeric = screen.getByRole("textbox", { name: /idle timeout/i });
        await user.clear(numeric);
        await user.type(numeric, "9999");
        await user.tab();
        expect(onChange).toHaveBeenCalledWith({ pool_idle_timeout: networkRangesStub.pool_idle_timeout_secs.max });
    });

    // The owner's range starts at the 0 sentinel; the numeric control must
    // start one past it (the checkbox owns "off"), so a typed 0 clamps to 1
    // rather than silently disabling eviction.
    it("the pool-idle numeric control never produces the 0 sentinel", async () => {
        const user = userEvent.setup();
        const onChange = vi.fn();
        render(
            <NetworkSection draft={{ ...baseDraft, pool_idle_timeout: 90 }} network={networkDefaultsStub} onChange={onChange} />,
        );
        const numeric = screen.getByRole("textbox", { name: /idle timeout/i });
        await user.clear(numeric);
        await user.type(numeric, "0");
        await user.tab();
        expect(onChange).toHaveBeenLastCalledWith({ pool_idle_timeout: networkRangesStub.pool_idle_timeout_secs.min + 1 });
    });

    // Inherited 0-sentinel: `pool_idle_timeout = 0` in config.toml with the
    // AppSettings field null (inherit) means eviction is OFF. The checkbox must
    // reflect the effective state, and the numeric input must not advertise a
    // "0" placeholder below its own minValue=1.
    describe("inheriting pool_idle_timeout_secs = 0 (eviction disabled by the base configuration)", () => {
        const inheritedOff = {
            ...networkDefaultsStub,
            effective: { ...effectiveNetworkStub, pool_idle_timeout_secs: 0 },
        };

        it("renders the checkbox unchecked and the numeric input disabled with no placeholder", () => {
            render(<NetworkSection draft={baseDraft} network={inheritedOff} onChange={vi.fn()} />);
            expect(screen.getByRole("checkbox", { name: /evict idle/i })).not.toBeChecked();
            const numeric = screen.getByRole("textbox", { name: /idle timeout/i });
            expect(numeric).toBeDisabled();
            expect(numeric).not.toHaveAttribute("placeholder");
        });

        // "Inherit" cannot re-enable eviction (the inherited value IS off), so the
        // toggle needs an explicit seconds value. It seeds the BUILT-IN default
        // served over IPC — not the control's lower bound (1 s ≈ no connection
        // reuse) and not a literal copy of the default (#611).
        it("checking the box seeds the built-in default served over IPC", () => {
            const onChange = vi.fn();
            render(<NetworkSection draft={baseDraft} network={inheritedOff} onChange={onChange} />);
            fireEvent.click(screen.getByRole("checkbox", { name: /evict idle/i }));
            expect(onChange).toHaveBeenCalledTimes(1);
            expect(onChange).toHaveBeenCalledWith({
                pool_idle_timeout: builtinNetworkStub.pool_idle_timeout_secs,
            });
        });

        it("an explicit draft value still wins over the inherited sentinel", () => {
            render(
                <NetworkSection draft={{ ...baseDraft, pool_idle_timeout: 90 }} network={inheritedOff} onChange={vi.fn()} />,
            );
            expect(screen.getByRole("checkbox", { name: /evict idle/i })).toBeChecked();
            expect(screen.getByRole("textbox", { name: /idle timeout/i })).toHaveValue("90");
        });
    });
});
