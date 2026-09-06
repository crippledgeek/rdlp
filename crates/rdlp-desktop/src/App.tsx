import { QueryClientProvider } from "@tanstack/react-query";
import { HotkeysProvider } from "@tanstack/react-hotkeys";
import { queryClient } from "./query/queryClient";
import { AppShell } from "./shell/AppShell";
import { ToastRegion } from "./components/ui/sonner";

// Tauri event listeners are registered at module scope in main.tsx, not here.
// Their lifetime is the window's, not any component's — see the comment there.

/** Root component — provides QueryClient and HotkeysProvider to the entire tree. */
function App() {
    return (
        <QueryClientProvider client={queryClient}>
            <HotkeysProvider>
                <AppShell />
                <ToastRegion />
            </HotkeysProvider>
        </QueryClientProvider>
    );
}

export default App;
