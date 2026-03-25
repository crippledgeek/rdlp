// Resilient thumbnail component with proxy fallback.
//
// Layer 1: Direct <img> with referrerPolicy="no-referrer".
// Layer 2: On load failure, fetches via Rust proxy_thumbnail command
//          which injects the correct Referer header for CDNs like CDN77.
// Layer 3: Placeholder <div> if proxy also fails.

import { useState, useCallback } from "react";
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
    const { data: proxyUrl, isError: proxyFailed, isPending: proxyPending } = useProxyThumbnail(src, directFailed);

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

    // Default: try direct load
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
