// Hook that tracks navigator.onLine status with event listeners.

import { useSyncExternalStore } from "react";

function subscribe(callback: () => void): () => void {
    window.addEventListener("online", callback);
    window.addEventListener("offline", callback);
    return () => {
        window.removeEventListener("online", callback);
        window.removeEventListener("offline", callback);
    };
}

function getSnapshot(): boolean {
    return navigator.onLine;
}

/** Returns true when the browser has network connectivity. */
export function useOnlineStatus(): boolean {
    return useSyncExternalStore(subscribe, getSnapshot, () => true);
}
