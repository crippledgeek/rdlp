import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Thumbnail } from "../Thumbnail";
import { setInvokeHandler, clearInvokeHandlers } from "../../test/tauri-mock";

function createWrapper() {
    const queryClient = new QueryClient({
        defaultOptions: { queries: { retry: false } },
    });
    return ({ children }: { children: React.ReactNode }) => (
        <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
}

describe("Thumbnail", () => {
    beforeEach(() => {
        clearInvokeHandlers();
    });

    it("renders direct img with no-referrer policy", () => {
        render(
            <Thumbnail src="https://example.com/thumb.jpg" alt="test" className="w-16 h-9" />,
            { wrapper: createWrapper() },
        );
        const img = screen.getByRole("img");
        expect(img).toHaveAttribute("src", "https://example.com/thumb.jpg");
        expect(img).toHaveAttribute("referrerPolicy", "no-referrer");
        expect(img).toHaveAttribute("loading", "lazy");
        expect(img).toHaveAttribute("alt", "test");
        expect(img).toHaveClass("w-16", "h-9");
    });

    it("renders placeholder when src is null", () => {
        const { container } = render(
            <Thumbnail src={null} alt="test" className="w-16 h-9" />,
            { wrapper: createWrapper() },
        );
        expect(screen.queryByRole("img")).toBeNull();
        const placeholder = container.firstElementChild;
        expect(placeholder?.tagName).toBe("DIV");
        expect(placeholder).toHaveClass("bg-muted");
    });

    it("renders placeholder when src is undefined", () => {
        const { container } = render(
            <Thumbnail src={undefined} alt="test" />,
            { wrapper: createWrapper() },
        );
        expect(screen.queryByRole("img")).toBeNull();
        expect(container.firstElementChild?.tagName).toBe("DIV");
    });

    it("falls back to proxy on direct load error", async () => {
        const fakeImageBytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer;
        setInvokeHandler("proxy_thumbnail", () => fakeImageBytes);

        // Spy on URL.createObjectURL / revokeObjectURL (preserves URL class)
        const mockBlobUrl = "blob:http://tauri.localhost/fake-uuid";
        vi.spyOn(URL, "createObjectURL").mockReturnValue(mockBlobUrl);
        vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});

        render(
            <Thumbnail src="https://cdn.example.com/thumb.jpg" alt="proxy test" />,
            { wrapper: createWrapper() },
        );

        // Direct img renders first
        const directImg = screen.getByRole("img");
        expect(directImg).toHaveAttribute("src", "https://cdn.example.com/thumb.jpg");

        // Simulate direct load failure
        await act(async () => {
            fireEvent.error(directImg);
        });

        // Wait for proxy query to resolve
        await waitFor(() => {
            const proxyImg = screen.getByRole("img");
            expect(proxyImg).toHaveAttribute("src", mockBlobUrl);
        });

        vi.restoreAllMocks();
    });

    it("shows placeholder when both direct and proxy fail", async () => {
        setInvokeHandler("proxy_thumbnail", () => {
            throw new Error("Proxy failed");
        });

        const { container } = render(
            <Thumbnail src="https://cdn.example.com/thumb.jpg" alt="fail test" />,
            { wrapper: createWrapper() },
        );

        const directImg = screen.getByRole("img");

        await act(async () => {
            fireEvent.error(directImg);
        });

        // Wait for proxy query to fail and placeholder to appear
        await waitFor(() => {
            expect(screen.queryByRole("img")).toBeNull();
            const placeholder = container.firstElementChild;
            expect(placeholder?.tagName).toBe("DIV");
            expect(placeholder).toHaveClass("bg-muted");
        });
    });

    it("revokes blob URL on unmount", async () => {
        const fakeImageBytes = new Uint8Array([0x89, 0x50]).buffer;
        setInvokeHandler("proxy_thumbnail", () => fakeImageBytes);

        const mockBlobUrl = "blob:http://tauri.localhost/cleanup-test";
        vi.spyOn(URL, "createObjectURL").mockReturnValue(mockBlobUrl);
        const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});

        const { unmount } = render(
            <Thumbnail src="https://cdn.example.com/thumb.jpg" alt="cleanup" />,
            { wrapper: createWrapper() },
        );

        // Trigger proxy path
        await act(async () => {
            fireEvent.error(screen.getByRole("img"));
        });

        await waitFor(() => {
            expect(screen.getByRole("img")).toHaveAttribute("src", mockBlobUrl);
        });

        // Unmount should revoke the blob URL
        unmount();
        expect(revokeObjectURL).toHaveBeenCalledWith(mockBlobUrl);

        vi.restoreAllMocks();
    });
});
