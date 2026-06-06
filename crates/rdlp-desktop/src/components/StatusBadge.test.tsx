import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusBadge } from "./StatusBadge";
import type { JobStatus } from "@/types";

describe("StatusBadge", () => {
    it("renders 'Processing' for processing status", () => {
        render(<StatusBadge status="processing" />);
        expect(screen.getByText("Processing")).toBeInTheDocument();
    });
    it("processing uses role=status, not alert", () => {
        render(<StatusBadge status="processing" />);
        expect(screen.getByRole("status")).toHaveTextContent("Processing");
    });
    it("maps every status to its label", () => {
        const cases: [JobStatus, string][] = [
            ["pending", "Queued"], ["running", "Downloading"], ["processing", "Processing"],
            ["completed", "Completed"], ["failed", "Failed"], ["cancelled", "Cancelled"],
        ];
        for (const [status, label] of cases) {
            const { unmount } = render(<StatusBadge status={status} />);
            expect(screen.getByText(label)).toBeInTheDocument();
            unmount();
        }
    });
});
