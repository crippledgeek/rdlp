import { render, screen } from "@/test/test-utils";
import userEvent from "@testing-library/user-event";
import {
    createColumnHelper,
    getCoreRowModel,
    useReactTable,
} from "@tanstack/react-table";
import { Table, TableBody } from "@/components/ui/table";
import { FormatGroupSection, type FormatGroup } from "./FormatGroupSection";
import type { FormatInfo } from "../types";

// -- Fixture data -------------------------------------------------------------

const fmt1080hls: FormatInfo = {
    format_id: "1080",
    ext: "mp4",
    format_note: "1080p",
    width: 1920,
    height: 1080,
    fps: 30,
    tbr: 2300,
    vcodec: "h264",
    acodec: "aac",
    filesize: 754,
    vbr: null,
    abr: null,
    asr: null,
    protocol: "hls",
    has_video: true,
    has_audio: true,
};

const fmt1080mp4: FormatInfo = {
    format_id: "1080mp4",
    ext: "mp4",
    format_note: "1080p",
    width: 1920,
    height: 1080,
    fps: null,
    tbr: null,
    vcodec: "h264",
    acodec: "aac",
    filesize: 837_900_000,
    vbr: null,
    abr: null,
    asr: null,
    protocol: "https",
    has_video: true,
    has_audio: true,
};

const fmt720: FormatInfo = {
    format_id: "720",
    ext: "mp4",
    format_note: "720p",
    width: 1280,
    height: 720,
    fps: 30,
    tbr: 1400,
    vcodec: "h264",
    acodec: "aac",
    filesize: 754,
    vbr: null,
    abr: null,
    asr: null,
    protocol: "hls",
    has_video: true,
    has_audio: true,
};

// -- Minimal column for rendering ---------------------------------------------

const col = createColumnHelper<FormatInfo>();
const columns = [
    col.accessor("format_id", { header: "ID", size: 100 }),
];

// -- Wrapper that creates a real TanStack table instance -----------------------

function TestHarness({
    data,
    selectedId = null,
    mergeId = null,
    exprMatches = new Set<string>(),
    onRowClick = vi.fn(),
}: {
    data: FormatInfo[];
    selectedId?: string | null;
    mergeId?: string | null;
    exprMatches?: Set<string>;
    onRowClick?: (id: string, e: React.MouseEvent) => void;
}) {
    const table = useReactTable({
        data,
        columns,
        getCoreRowModel: getCoreRowModel(),
    });

    const group: FormatGroup = {
        label: "Video + Audio",
        rows: table.getRowModel().rows,
    };

    return (
        <Table>
            <TableBody>
                <FormatGroupSection
                    group={group}
                    columnCount={1}
                    selectedId={selectedId}
                    mergeId={mergeId}
                    exprMatches={exprMatches}
                    onRowClick={onRowClick}
                />
            </TableBody>
        </Table>
    );
}

// -- Tests --------------------------------------------------------------------

describe("FormatGroupSection", () => {
    it("renders the group label", () => {
        render(<TestHarness data={[fmt1080hls]} />);
        expect(screen.getByText("Video + Audio")).toBeInTheDocument();
    });

    it("renders format rows", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} />);
        expect(screen.getByText("1080")).toBeInTheDocument();
        expect(screen.getByText("720")).toBeInTheDocument();
    });

    it("applies selected styling only to the selected row", () => {
        render(<TestHarness data={[fmt1080hls, fmt1080mp4, fmt720]} selectedId="1080" />);
        const rows = screen.getAllByRole("row");
        // row 0 = group header, row 1 = 1080 (selected), row 2 = 1080mp4, row 3 = 720
        expect(rows[1].className).toContain("bg-primary/30");
        expect(rows[1].className).toContain("border-l-primary");
        // Adjacent unselected rows must NOT have selection classes
        expect(rows[2].className).not.toContain("bg-primary/30");
        expect(rows[2].className).not.toContain("border-l-primary");
        expect(rows[3].className).not.toContain("bg-primary/30");
    });

    it("does not apply zebra stripe to selected row", () => {
        // Row at index 1 (odd) would normally get zebra stripe
        render(<TestHarness data={[fmt1080hls, fmt1080mp4]} selectedId="1080mp4" />);
        const rows = screen.getAllByRole("row");
        // row 2 = fmt1080mp4 (index 1, odd, selected)
        expect(rows[2].className).not.toContain("bg-foreground/[0.02]");
        expect(rows[2].className).toContain("bg-primary/30");
    });

    it("applies merge styling to merge row", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} selectedId="1080" mergeId="720" />);
        const rows = screen.getAllByRole("row");
        // row 2 = 720 (merge)
        expect(rows[2].className).toContain("bg-foreground/15");
        expect(rows[2].className).toContain("border-l-foreground/40");
    });

    it("does not apply zebra stripe to merge row", () => {
        render(<TestHarness data={[fmt1080hls, fmt1080mp4]} selectedId="1080" mergeId="1080mp4" />);
        const rows = screen.getAllByRole("row");
        // row 2 = fmt1080mp4 (merge)
        expect(rows[2].className).not.toContain("bg-foreground/[0.02]");
        expect(rows[2].className).toContain("bg-foreground/15");
    });

    it("fires onRowClick with the format id", async () => {
        const onClick = vi.fn();
        const user = userEvent.setup();
        render(<TestHarness data={[fmt1080hls]} onRowClick={onClick} />);
        const rows = screen.getAllByRole("row");
        await user.click(rows[1]); // row 0 = header, row 1 = data
        expect(onClick).toHaveBeenCalledWith("1080", expect.any(Object));
    });

    it("applies text-foreground to selected row cells", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} selectedId="1080" />);
        const cells = screen.getAllByRole("cell");
        // cell 0 = group header, cell 1 = "1080" (selected), cell 2 = "720" (not selected)
        expect(cells[1].className).toContain("text-foreground");
        expect(cells[2].className).toContain("text-foreground/60");
    });

    it("applies zebra stripe to odd unselected rows", () => {
        render(<TestHarness data={[fmt1080hls, fmt1080mp4, fmt720]} />);
        const rows = screen.getAllByRole("row");
        // row 1 = index 0 (even) → bg-transparent, row 2 = index 1 (odd) → zebra
        expect(rows[1].className).toContain("bg-transparent");
        expect(rows[2].className).toContain("bg-foreground/[0.02]");
    });

    it("applies expression match background to matched unselected row", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} exprMatches={new Set(["720"])} />);
        const rows = screen.getAllByRole("row");
        // row 1 = 1080 (no match), row 2 = 720 (match)
        expect(rows[1].className).not.toContain("bg-yellow-500/5");
        expect(rows[2].className).toContain("bg-yellow-500/5");
    });

    it("applies text-yellow-500 to expression-matched cells", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} exprMatches={new Set(["720"])} />);
        const cells = screen.getAllByRole("cell");
        // cell 0 = header, cell 1 = 1080 (no match), cell 2 = 720 (match)
        expect(cells[1].className).toContain("text-foreground/60");
        expect(cells[2].className).toContain("text-yellow-500");
    });

    it("suppresses expression match styling when row is selected", () => {
        render(
            <TestHarness data={[fmt1080hls]} selectedId="1080" exprMatches={new Set(["1080"])} />,
        );
        const rows = screen.getAllByRole("row");
        const cells = screen.getAllByRole("cell");
        // Selected takes priority over expr match
        expect(rows[1].className).toContain("bg-primary/30");
        expect(rows[1].className).not.toContain("bg-yellow-500/5");
        expect(cells[1].className).toContain("text-foreground");
        expect(cells[1].className).not.toContain("text-yellow-500");
    });

    it("suppresses expression match styling when row is merge target", () => {
        render(
            <TestHarness
                data={[fmt1080hls, fmt720]}
                selectedId="1080"
                mergeId="720"
                exprMatches={new Set(["720"])}
            />,
        );
        const rows = screen.getAllByRole("row");
        const cells = screen.getAllByRole("cell");
        // Merge takes priority over expr match
        expect(rows[2].className).toContain("bg-foreground/15");
        expect(rows[2].className).not.toContain("bg-yellow-500/5");
        expect(cells[2].className).toContain("text-foreground");
        expect(cells[2].className).not.toContain("text-yellow-500");
    });

    it("gives unselected rows default hover and selected rows dedicated hover", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} selectedId="1080" />);
        const rows = screen.getAllByRole("row");
        // Selected row: dedicated hover
        expect(rows[1].className).toContain("hover:bg-primary/35");
        expect(rows[1].className).not.toContain("hover:bg-foreground/[0.06]");
        // Unselected row: default hover
        expect(rows[2].className).toContain("hover:bg-foreground/[0.06]");
        expect(rows[2].className).not.toContain("hover:bg-primary/35");
    });

    it("gives merge row its own hover class", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} selectedId="1080" mergeId="720" />);
        const rows = screen.getAllByRole("row");
        expect(rows[2].className).toContain("hover:bg-foreground/18");
        expect(rows[2].className).not.toContain("hover:bg-foreground/[0.06]");
    });

    it("applies text-foreground to merge row cells", () => {
        render(<TestHarness data={[fmt1080hls, fmt720]} selectedId="1080" mergeId="720" />);
        const cells = screen.getAllByRole("cell");
        // cell 1 = selected, cell 2 = merge — both get text-foreground
        expect(cells[1].className).toContain("text-foreground");
        expect(cells[2].className).toContain("text-foreground");
    });
});
