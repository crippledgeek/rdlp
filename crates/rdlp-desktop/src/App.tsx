import { useEffect } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { HotkeysProvider } from "@tanstack/react-hotkeys";
import { queryClient } from "./query/queryClient";
import { registerDownloadEvents } from "./events/registerDownloadEvents";
import { AppShell } from "./shell/AppShell";
import { Toaster } from "./components/ui/sonner";

/** Inner component that uses hooks requiring QueryClientProvider. */
function AppContent() {
    // Register all Tauri download event listeners (single wiring point)
    useEffect(() => {
        return registerDownloadEvents(queryClient);
    }, []);

    return <AppShell />;
}

/** Root component — provides QueryClient and HotkeysProvider to the entire tree. */
function App() {
    return (
        <QueryClientProvider client={queryClient}>
            <HotkeysProvider>
                <AppContent />
                <Toaster />
            </HotkeysProvider>
        </QueryClientProvider>
    );
}

export default App;
