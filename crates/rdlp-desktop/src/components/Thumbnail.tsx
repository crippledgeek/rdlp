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

/**
 * Fetch a thumbnail via the Rust proxy, returning a Blob URL.
 *
 * Uses raw `invoke` (not `invokeTyped`) because `tauri::ipc::Response`
 * returns binary data as an ArrayBuffer, bypassing JSON serialization.
 * The query is dormant (`enabled: false`) until the direct <img> fails.
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
    const [directFailed, setDirectFailed] = useState(false);
    const imgRef = useRef<HTMLImageElement>(null);
    const { data: proxyUrl, isError: proxyFailed } = useProxyThumbnail(src, directFailed);

    // Detect broken images that WebKitGTK doesn't fire onError for.
    // Check naturalWidth after mount — a broken image has naturalWidth === 0.
    const handleLoad = useCallback(() => {
        if (imgRef.current && imgRef.current.naturalWidth === 0) {
            setDirectFailed(true);
        }
    }, []);

    const handleError = useCallback(() => setDirectFailed(true), []);

    // Also check on mount in case the image was already cached/broken before
    // React attached event handlers (WebKitGTK race condition).
    useEffect(() => {
        const img = imgRef.current;
        if (img && img.complete) {
            if (img.naturalWidth === 0) {
                setDirectFailed(true);
            }
        }
    }, [src]);

    // Revoke Blob URL on unmount or when proxyUrl changes.
    useEffect(() => {
        return () => {
            if (proxyUrl) URL.revokeObjectURL(proxyUrl);
        };
    }, [proxyUrl]);

    // No source, or both layers failed → placeholder
    if (!src || (directFailed && proxyFailed)) {
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
