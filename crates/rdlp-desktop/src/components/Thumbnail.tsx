// Resilient thumbnail component with proxy fallback.
//
// Strategy:
//   - External HTTPS URLs are always routed through the Rust proxy_thumbnail
//     command. WebKitGTK (Tauri's Linux webview) has race conditions with
//     cross-origin <img> loads under the `referrerPolicy="no-referrer"` policy
//     used to avoid leaking the app origin, especially with the DMA-BUF
//     renderer disabled (required on NVIDIA proprietary drivers). Always
//     proxying side-steps that WebKit quirk at the cost of one extra IPC
//     round-trip per thumbnail.
//   - Same-origin (blob:, data:, http://localhost) URLs are rendered directly.
//   - If the proxy itself fails, show a placeholder.

import { useCallback, useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "@/lib/utils";
import { queryKeys } from "@/query/queryKeys";

/** URLs known to fail direct load — skip straight to proxy on remount. */
const directFailCache = new Set<string>();

/** Returns true when the URL points at an external origin that should go
 *  through the Rust proxy (HTTPS, non-localhost). */
function shouldProxy(url: string): boolean {
    if (url.startsWith("blob:") || url.startsWith("data:")) return false;
    if (url.startsWith("http://")) return false; // dev assets, localhost
    try {
        const u = new URL(url);
        if (u.protocol !== "https:") return false;
        // Tauri asset: / localhost-style hosts — render direct
        if (u.hostname === "localhost" || u.hostname === "127.0.0.1") return false;
        return true;
    } catch {
        return false;
    }
}

/**
 * Fetch a thumbnail via the Rust proxy, returning a Blob URL.
 *
 * Uses raw `invoke` (not `invokeTyped`) because `tauri::ipc::Response`
 * returns binary data as an ArrayBuffer, bypassing JSON serialization.
 * The query is dormant (`enabled: false`) until the direct <img> fails.
 *
 * Blob URLs are NOT revoked — they are cached by TanStack Query and must
 * remain valid across component remounts (e.g. after table sort). The
 * browser reclaims them when the page unloads.
 */
function useProxyThumbnail(url: string | null | undefined, enabled: boolean) {
    return useQuery({
        queryKey: queryKeys.thumbnail.proxy(url),
        queryFn: async () => {
            const bytes = await invoke<ArrayBuffer>("proxy_thumbnail", { url });
            return URL.createObjectURL(new Blob([new Uint8Array(bytes)]));
        },
        enabled: enabled && !!url,
        staleTime: Infinity,
        gcTime: 5 * 60 * 1000,
        retry: false,
    });
}

interface ThumbnailProps {
    src: string | null | undefined;
    alt: string;
    className?: string;
    decoding?: "async" | "sync" | "auto";
}

/** Thumbnail with automatic proxy fallback for CDNs requiring Referer. */
export function Thumbnail({ src, alt, className, decoding }: ThumbnailProps) {
    // External HTTPS URLs always go through the proxy on WebKitGTK (see module
    // header comment for rationale). Same-origin/local URLs try direct first,
    // with proxy fallback on failure.
    const useProxyFromStart = useMemo(() => !!src && shouldProxy(src), [src]);
    const initialDirectFailed = useMemo(
        () => useProxyFromStart || (!!src && directFailCache.has(src)),
        [useProxyFromStart, src],
    );
    const [directFailed, setDirectFailed] = useState(initialDirectFailed);
    const {
        data: proxyUrl,
        isError: proxyFailed,
        isPending: proxyPending,
        error: proxyErr,
    } = useProxyThumbnail(src, directFailed);

    // Diagnostic logging — emit one line per state transition so we can tell
    // whether the proxy is running, succeeding, or the blob URL isn't painting.
    // TODO: remove once the rendering quirk is root-caused.
    useEffect(() => {
        if (!src) return;
        if (import.meta.env.DEV) {
            // eslint-disable-next-line no-console
            console.debug("[Thumbnail]", {
                src: src.slice(0, 80),
                useProxyFromStart,
                directFailed,
                proxyPending,
                proxyFailed,
                proxyErr: proxyErr instanceof Error ? proxyErr.message : String(proxyErr ?? ""),
                proxyUrl: proxyUrl?.slice(0, 60),
            });
        }
    }, [src, useProxyFromStart, directFailed, proxyPending, proxyFailed, proxyErr, proxyUrl]);

    // Ref callback: runs when the img element mounts or src changes.
    // Handles the WebKitGTK race condition where img.complete is true before
    // React attaches onLoad/onError. No useEffect needed.
    const imgRefCallback = useCallback((img: HTMLImageElement | null) => {
        if (img && img.complete && img.naturalWidth === 0) {
            if (src) directFailCache.add(src);
            setDirectFailed(true);
        }
    }, [src]);

    const handleLoad = useCallback((e: React.SyntheticEvent<HTMLImageElement>) => {
        if (e.currentTarget.naturalWidth === 0) {
            if (src) directFailCache.add(src);
            setDirectFailed(true);
        }
    }, [src]);

    const handleError = useCallback(() => {
        if (src) directFailCache.add(src);
        setDirectFailed(true);
    }, [src]);

    // No source → placeholder
    if (!src) {
        return <div className={cn("bg-muted", className)} />;
    }

    // Direct failed, proxy loading → placeholder (not the broken direct <img>)
    if (directFailed && proxyPending) {
        return <div className={cn("bg-muted animate-pulse", className)} />;
    }

    // Direct failed, proxy also failed → placeholder
    if (directFailed && proxyFailed) {
        return <div className={cn("bg-muted", className)} />;
    }

    // Proxy succeeded → show proxied image
    if (directFailed && proxyUrl) {
        return <img src={proxyUrl} alt={alt} className={className} loading="lazy" decoding={decoding} />;
    }

    // Default: try direct load (same-origin / local URLs)
    return (
        <img
            ref={imgRefCallback}
            src={src}
            alt={alt}
            className={className}
            loading="lazy"
            decoding={decoding}
            referrerPolicy="no-referrer"
            onLoad={handleLoad}
            onError={handleError}
        />
    );
}
