// Resilient thumbnail component with proxy fallback.
//
// Layer 1: Direct <img> with referrerPolicy="no-referrer".
// Layer 2: On load failure, fetches via Rust proxy_thumbnail command
//          which injects the correct Referer header for CDNs like CDN77.
// Layer 3: Placeholder <div> if proxy also fails.

import { useState, useEffect } from "react";
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
    const { data: proxyUrl, isError: proxyFailed } = useProxyThumbnail(src, directFailed);

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
            src={src}
            alt={alt}
            className={className}
            loading="lazy"
            decoding={decoding}
            referrerPolicy="no-referrer"
            onError={() => setDirectFailed(true)}
        />
    );
}
