// Resilient thumbnail component with proxy fallback.
//
// Layer 1: Direct <img> with referrerPolicy="no-referrer".
// Layer 2: On load failure, fetches via Rust proxy_thumbnail command
//          which injects the correct Referer header for CDNs like CDN77.
// Layer 3: Placeholder <div> if proxy also fails.

import { useState, useEffect, useCallback, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "@/lib/utils";
import { queryKeys } from "@/query/queryKeys";

/** URLs known to fail direct load — skip straight to proxy on remount. */
const directFailCache = new Set<string>();

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
    // Check module-level cache so remounted components skip the direct attempt
    const [directFailed, setDirectFailed] = useState(() => !!src && directFailCache.has(src));
    const imgRef = useRef<HTMLImageElement>(null);
    const { data: proxyUrl, isError: proxyFailed, isPending: proxyPending } = useProxyThumbnail(src, directFailed);

    // Detect broken images that WebKitGTK doesn't fire onError for.
    // Check naturalWidth after mount — a broken image has naturalWidth === 0.
    const handleLoad = useCallback(() => {
        if (imgRef.current && imgRef.current.naturalWidth === 0) {
            if (src) directFailCache.add(src);
            setDirectFailed(true);
        }
    }, [src]);

    const handleError = useCallback(() => {
        if (src) directFailCache.add(src);
        setDirectFailed(true);
    }, [src]);

    // Also check on mount in case the image was already cached/broken before
    // React attached event handlers (WebKitGTK race condition).
    useEffect(() => {
        const img = imgRef.current;
        if (img && img.complete) {
            if (img.naturalWidth === 0) {
                if (src) directFailCache.add(src);
                setDirectFailed(true);
            }
        }
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

    // Default: try direct load
    return (
        <img
            ref={imgRef}
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
