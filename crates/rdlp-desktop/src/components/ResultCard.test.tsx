import { render, screen, fireEvent, createTestQueryClient } from "@/test/test-utils";
import { ResultCard } from "./ResultCard";
import { queryKeys } from "../query/queryKeys";
import type { SearchResultPreview } from "../types";

const baseResult: SearchResultPreview = {
    video_url: "https://example.com/video.mp4",
    title: "Sample Video Title",
    thumbnail_url: "https://example.com/thumb.jpg",
    duration: 185,
    view_count: 12500,
    upload_date: null,
};

/** Pre-seed settings so ResultCard renders synchronously. */
function seededClient() {
    const qc = createTestQueryClient();
    qc.setQueryData(queryKeys.settings(), null);
    return qc;
}

describe("ResultCard", () => {
    it("renders the video title", () => {
        render(
            <ResultCard
                result={baseResult}
                onDownload={vi.fn()}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={vi.fn()}
            />,
            { queryClient: seededClient() },
        );
        expect(screen.getByRole("heading", { name: "Sample Video Title" })).toBeInTheDocument();
    });

    it("renders thumbnail image with alt text", () => {
        render(
            <ResultCard
                result={baseResult}
                onDownload={vi.fn()}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={vi.fn()}
            />,
            { queryClient: seededClient() },
        );
        const img = screen.getByRole("img", { name: "Sample Video Title" });
        expect(img).toHaveAttribute("src", "https://example.com/thumb.jpg");
    });

    it("shows 'No thumbnail' placeholder when thumbnail_url is null", () => {
        render(
            <ResultCard
                result={{ ...baseResult, thumbnail_url: null }}
                onDownload={vi.fn()}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={vi.fn()}
            />,
            { queryClient: seededClient() },
        );
        expect(screen.getByText(/no thumbnail/i)).toBeInTheDocument();
    });

    it("displays formatted duration", () => {
        render(
            <ResultCard
                result={baseResult}
                onDownload={vi.fn()}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={vi.fn()}
            />,
            { queryClient: seededClient() },
        );
        // 185s = 3:05
        expect(screen.getByText("3:05")).toBeInTheDocument();
    });

    it("displays formatted view count", () => {
        render(
            <ResultCard
                result={baseResult}
                onDownload={vi.fn()}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={vi.fn()}
            />,
            { queryClient: seededClient() },
        );
        expect(screen.getByText("12.5K views")).toBeInTheDocument();
    });

    it("calls onDownload with video_url and title when Download button is clicked", () => {
        const onDownload = vi.fn();
        render(
            <ResultCard
                result={baseResult}
                onDownload={onDownload}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={vi.fn()}
            />,
            { queryClient: seededClient() },
        );
        fireEvent.click(screen.getByRole("button", { name: /^download$/i }));
        expect(onDownload).toHaveBeenCalledWith(
            "https://example.com/video.mp4",
            "Sample Video Title",
        );
    });

    it("calls onOpenFormatDialog when Choose Format is clicked", () => {
        const onOpenFormatDialog = vi.fn();
        render(
            <ResultCard
                result={baseResult}
                onDownload={vi.fn()}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={onOpenFormatDialog}
            />,
            { queryClient: seededClient() },
        );
        fireEvent.click(screen.getByRole("button", { name: /choose format/i }));
        expect(onOpenFormatDialog).toHaveBeenCalledWith("https://example.com/video.mp4");
    });

    it("does not display view count when view_count is null", () => {
        render(
            <ResultCard
                result={{ ...baseResult, view_count: null }}
                onDownload={vi.fn()}
                onDownloadWithOptions={vi.fn()}
                onOpenFormatDialog={vi.fn()}
            />,
            { queryClient: seededClient() },
        );
        expect(screen.queryByText(/views/i)).not.toBeInTheDocument();
    });
});
