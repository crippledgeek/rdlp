import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@/test/test-utils";
import { NormalizationSection } from "./NormalizationSection";
import { effectiveNormalizeLoudStub, effectiveNormalizeStub, loudnormPresetsStub } from "@/test/effectiveNormalizeStub";
import { appSettingsStub } from "@/test/appSettingsStub";
import type { AppSettings } from "@/types";

const baseDraft: AppSettings = {
    ...appSettingsStub,
    normalize_audio: true,
    loudnorm: true,
    normalize_boost: true,
};

// #611: the placeholder is the "inherit" hint, so it must state the value the
// engine actually uses for the DRAFT'S preset. That arrives over IPC as the
// `EffectiveNormalize` payload (owner: `rdlp_types::EffectiveNormalize` +
// `LoudnormPreset::targets()`, resolved by `PostProcess::effective_normalize`).
// The stubs' values differ from the real defaults, so a leftover literal fails.
describe("NormalizationSection — placeholders derive from the effective-normalize payload", () => {
    it("every loudnorm target placeholder is the payload's value", () => {
        render(<NormalizationSection draft={baseDraft} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={vi.fn()} />);
        const stub = effectiveNormalizeStub;
        expect(screen.getByLabelText(/loudness \(lufs\)/i)).toHaveAttribute("placeholder", String(stub.targets.integrated_lufs));
        expect(screen.getByLabelText(/true peak \(dbtp\)/i)).toHaveAttribute("placeholder", String(stub.targets.true_peak_dbtp));
        expect(screen.getByLabelText(/range \(lu\)/i)).toHaveAttribute("placeholder", String(stub.targets.range_lu));
        expect(screen.getByLabelText(/boost gain/i)).toHaveAttribute("placeholder", String(stub.boost_gain_db));
    });

    it("the peak target placeholder is the payload's value in peak mode", () => {
        const draft = { ...baseDraft, loudnorm: false };
        render(<NormalizationSection draft={draft} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={vi.fn()} />);
        expect(screen.getByLabelText(/peak target/i)).toHaveAttribute(
            "placeholder",
            String(effectiveNormalizeStub.peak_target_db),
        );
    });

    // The ranges have ONE owner (`rdlp_types::EffectiveNormalize::*_RANGE`),
    // enforced behind `update_settings`; a client-side `min`/`max` would be a
    // second copy of it — and the copies had already drifted (#611 review).
    it("no target input carries a client-side min/max", () => {
        const { rerender } = render(
            <NormalizationSection draft={{ ...baseDraft, loudnorm: false }} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={vi.fn()} />,
        );
        const peak = screen.getByLabelText(/peak target/i);
        expect(peak).not.toHaveAttribute("min");
        expect(peak).not.toHaveAttribute("max");
        rerender(<NormalizationSection draft={baseDraft} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={vi.fn()} />);
        for (const label of [/loudness \(lufs\)/i, /true peak \(dbtp\)/i, /range \(lu\)/i, /boost gain/i]) {
            const input = screen.getByLabelText(label);
            expect(input).not.toHaveAttribute("min");
            expect(input).not.toHaveAttribute("max");
        }
    });

    // The I/TP/LRA defaults are preset-dependent. Before #611 the hints were
    // Streaming-only literals, so they were WRONG under Loud/Broadcast.
    it("switching the preset changes the loudnorm target placeholders", () => {
        const { rerender } = render(
            <NormalizationSection draft={baseDraft} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={vi.fn()} />,
        );
        expect(screen.getByLabelText(/loudness \(lufs\)/i)).toHaveAttribute(
            "placeholder",
            String(effectiveNormalizeStub.targets.integrated_lufs),
        );
        rerender(
            <NormalizationSection
                draft={{ ...baseDraft, loudnorm_preset: "loud" }}
                effective={effectiveNormalizeLoudStub}
                presets={loudnormPresetsStub}
                onChange={vi.fn()}
            />,
        );
        expect(screen.getByLabelText(/loudness \(lufs\)/i)).toHaveAttribute(
            "placeholder",
            String(effectiveNormalizeLoudStub.targets.integrated_lufs),
        );
        expect(screen.getByLabelText(/range \(lu\)/i)).toHaveAttribute(
            "placeholder",
            String(effectiveNormalizeLoudStub.targets.range_lu),
        );
    });

    // With no preset chosen, the Select's inherit entry names the preset the
    // engine actually resolved to — from the payload, not a typed "Streaming".
    it("the inherit entry of the preset Select names the payload's resolved preset", () => {
        render(<NormalizationSection draft={baseDraft} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={vi.fn()} />);
        expect(screen.getByRole("combobox")).toHaveTextContent(/default \(broadcast\)/i);
    });

    // The per-item numbers come from the `loudnorm_presets` payload (owner:
    // `LoudnormPreset::targets`), not a typed copy — the stub's values differ
    // from the real targets, so a literal label fails.
    it("every preset item is labelled with the payload's integrated-loudness target", () => {
        render(<NormalizationSection draft={baseDraft} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={vi.fn()} />);
        fireEvent.pointerDown(screen.getByRole("combobox"));
        for (const { preset, targets } of loudnormPresetsStub) {
            const expected = new RegExp(`^${preset} \\(${targets.integrated_lufs} LUFS\\)$`, "i");
            expect(screen.getByRole("option", { name: expected })).toBeInTheDocument();
        }
        // Exactly the catalogue plus the inherit entry — no hand-listed extras.
        expect(screen.getAllByRole("option")).toHaveLength(loudnormPresetsStub.length + 1);
    });

    it("choosing a preset commits its wire value; choosing the inherit entry commits null", () => {
        const onChange = vi.fn();
        render(<NormalizationSection draft={baseDraft} effective={effectiveNormalizeStub} presets={loudnormPresetsStub} onChange={onChange} />);
        fireEvent.pointerDown(screen.getByRole("combobox"));
        fireEvent.click(screen.getByRole("option", { name: /^loud \(/i }));
        expect(onChange).toHaveBeenCalledWith({ loudnorm_preset: "loud" });
        fireEvent.pointerDown(screen.getByRole("combobox"));
        fireEvent.click(screen.getByRole("option", { name: /default/i }));
        expect(onChange).toHaveBeenCalledWith({ loudnorm_preset: null });
    });
});
