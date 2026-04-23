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

    it("renders direct img with no-referrer policy for local URLs", () => {
        // Same-origin / local URLs bypass the proxy and render directly.
        render(
            <Thumbnail src="http://localhost/thumb.jpg" alt="test" className="w-16 h-9" />,
            { wrapper: createWrapper() },
        );
        const img = screen.getByRole("img");
        expect(img).toHaveAttribute("src", "http://localhost/thumb.jpg");
        expect(img).toHaveAttribute("referrerPolicy", "no-referrer");
        expect(img).toHaveAttribute("loading", "lazy");
        expect(img).toHaveAttribute("alt", "test");
        expect(img).toHaveClass("w-16", "h-9");
    });

    it("routes external HTTPS URLs straight through the Rust proxy", async () => {
        // External HTTPS URLs skip the direct <img> attempt entirely because
        // WebKitGTK has known issues with cross-origin loads under
        // referrerPolicy=no-referrer on NVIDIA with DMA-BUF disabled.
        const fakeImageBytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer;
        setInvokeHandler("proxy_thumbnail", () => fakeImageBytes);

        const mockBlobUrl = "blob:http://tauri.localhost/proxy-first";
        vi.spyOn(URL, "createObjectURL").mockReturnValue(mockBlobUrl);
        vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});

        render(
            <Thumbnail src="https://cdn.example.com/thumb.jpg" alt="proxy-first" />,
            { wrapper: createWrapper() },
        );

        // Directly renders via proxy (no intermediate direct <img> attempt).
        await waitFor(() => {
            const img = screen.getByRole("img");
            expect(img).toHaveAttribute("src", mockBlobUrl);
        });

        vi.restoreAllMocks();
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

    it("falls back to proxy when a local direct load errors", async () => {
        // Local/same-origin URLs still use the direct-then-fallback flow.
        const fakeImageBytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer;
        setInvokeHandler("proxy_thumbnail", () => fakeImageBytes);

        const mockBlobUrl = "blob:http://tauri.localhost/fake-uuid";
        vi.spyOn(URL, "createObjectURL").mockReturnValue(mockBlobUrl);
        vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});

        render(
            <Thumbnail src="http://localhost/thumb.jpg" alt="proxy test" />,
            { wrapper: createWrapper() },
        );

        const directImg = screen.getByRole("img");
        expect(directImg).toHaveAttribute("src", "http://localhost/thumb.jpg");

        await act(async () => {
            fireEvent.error(directImg);
        });

        await waitFor(() => {
            const proxyImg = screen.getByRole("img");
            expect(proxyImg).toHaveAttribute("src", mockBlobUrl);
        });

        vi.restoreAllMocks();
    });

    it("shows placeholder when proxy fails for an external URL", async () => {
        setInvokeHandler("proxy_thumbnail", () => {
            throw new Error("Proxy failed");
        });

        const { container } = render(
            <Thumbnail src="https://cdn.example.com/fail-both.jpg" alt="fail test" />,
            { wrapper: createWrapper() },
        );

        // External URL → proxy is used straight away; when the proxy query
        // fails, the placeholder appears without any prior direct <img>.
        await waitFor(() => {
            expect(screen.queryByRole("img")).toBeNull();
            const placeholder = container.firstElementChild;
            expect(placeholder?.tagName).toBe("DIV");
            expect(placeholder).toHaveClass("bg-muted");
        });
    });

    it("does not revoke blob URL on unmount (cached by TanStack Query)", async () => {
        const fakeImageBytes = new Uint8Array([0x89, 0x50]).buffer;
        setInvokeHandler("proxy_thumbnail", () => fakeImageBytes);

        const mockBlobUrl = "blob:http://tauri.localhost/cleanup-test";
        vi.spyOn(URL, "createObjectURL").mockReturnValue(mockBlobUrl);
        const revokeObjectURL = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});

        const { unmount } = render(
            <Thumbnail src="https://cdn.example.com/no-revoke.jpg" alt="cleanup" />,
            { wrapper: createWrapper() },
        );

        // External URL → proxy used immediately, blob URL appears after the
        // query resolves.
        await waitFor(() => {
            expect(screen.getByRole("img")).toHaveAttribute("src", mockBlobUrl);
        });

        // Unmount should NOT revoke — blob URL is cached by TanStack Query
        // and must remain valid for remounted components (e.g. after sort)
        unmount();
        expect(revokeObjectURL).not.toHaveBeenCalled();

        vi.restoreAllMocks();
    });
});
