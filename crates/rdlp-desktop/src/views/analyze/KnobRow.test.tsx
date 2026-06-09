import { render, screen } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { KnobRow } from "./KnobRow";
import type { SpeedKnob } from "@/types";

describe("KnobRow", () => {
    it("renders a Choice knob label", () => {
        const knob: SpeedKnob = {
            kind: "choice",
            field: "preset",
            label: "Preset",
            choices: ["faster", "medium"],
            default: "medium",
        };
        render(<KnobRow knob={knob} value="" onChange={vi.fn()} onBlur={vi.fn()} />);
        expect(screen.getByText("Preset")).toBeInTheDocument();
    });

    it("renders an Int knob as a bounded number input", () => {
        const knob: SpeedKnob = {
            kind: "int",
            field: "cpuUsed",
            label: "CPU used",
            min: -8,
            max: 8,
            default: 1,
        };
        render(<KnobRow knob={knob} value="" onChange={vi.fn()} onBlur={vi.fn()} />);
        const input = screen.getByPlaceholderText("1") as HTMLInputElement;
        expect(input.min).toBe("-8");
        expect(input.max).toBe("8");
    });

    it("renders an Int knob label", () => {
        const knob: SpeedKnob = {
            kind: "int",
            field: "cpuUsed",
            label: "CPU used",
            min: -8,
            max: 8,
            default: 1,
        };
        render(<KnobRow knob={knob} value="" onChange={vi.fn()} onBlur={vi.fn()} />);
        expect(screen.getByText("CPU used")).toBeInTheDocument();
    });

    it("passes value to the Int knob input", () => {
        const knob: SpeedKnob = {
            kind: "int",
            field: "speedLevel",
            label: "Speed Level",
            min: 0,
            max: 9,
            default: 4,
        };
        render(<KnobRow knob={knob} value="3" onChange={vi.fn()} onBlur={vi.fn()} />);
        const input = screen.getByDisplayValue("3") as HTMLInputElement;
        expect(input).toBeInTheDocument();
    });

    it("renders all choice options including the Default sentinel", () => {
        const knob: SpeedKnob = {
            kind: "choice",
            field: "preset",
            label: "Preset",
            choices: ["faster", "medium", "slower"],
            default: "medium",
        };
        render(<KnobRow knob={knob} value="" onChange={vi.fn()} onBlur={vi.fn()} />);
        // Open the select to make options visible
        fireEvent.pointerDown(screen.getByRole("combobox"));
        expect(screen.getByText("Default (medium)")).toBeInTheDocument();
        expect(screen.getByText("faster")).toBeInTheDocument();
        expect(screen.getByText("medium")).toBeInTheDocument();
        expect(screen.getByText("slower")).toBeInTheDocument();
    });

    it("calls onChange with the chosen value when a choice option is selected", () => {
        const knob: SpeedKnob = {
            kind: "choice",
            field: "preset",
            label: "Preset",
            choices: ["faster", "medium", "slower"],
            default: "medium",
        };
        const onChange = vi.fn();
        render(<KnobRow knob={knob} value="" onChange={onChange} onBlur={vi.fn()} />);
        // Open the select
        fireEvent.pointerDown(screen.getByRole("combobox"));
        // Click the "faster" option
        fireEvent.click(screen.getByRole("option", { name: "faster" }));
        expect(onChange).toHaveBeenCalledWith("faster");
    });

    it("calls onChange with empty string when the Default sentinel is selected", () => {
        const knob: SpeedKnob = {
            kind: "choice",
            field: "preset",
            label: "Preset",
            choices: ["faster", "medium"],
            default: "medium",
        };
        const onChange = vi.fn();
        render(<KnobRow knob={knob} value="faster" onChange={onChange} onBlur={vi.fn()} />);
        // Open the select
        fireEvent.pointerDown(screen.getByRole("combobox"));
        // Click the Default sentinel
        fireEvent.click(screen.getByText("Default (medium)"));
        // NONE_SENTINEL → converted back to ""
        expect(onChange).toHaveBeenCalledWith("");
    });
});
