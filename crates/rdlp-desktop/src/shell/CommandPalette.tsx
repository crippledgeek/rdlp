// CommandPalette: Ctrl+Shift+P overlay using shadcn Command (cmdk).
// Sections: Navigation, Actions.

import { useStore } from "@tanstack/react-store";
import { ModalOverlay, Modal, Dialog } from "react-aria-components";
import {
    Command,
    CommandEmpty,
    CommandGroup,
    CommandInput,
    CommandItem,
    CommandList,
    CommandSeparator,
} from "@/components/ui/command";
import { uiStore, setCommandPaletteOpen, setView, toggleNavPanel, toggleBottomDrawer } from "@/stores/uiStore";
import { Search, ArrowDownToLine, Clock, Settings, PanelLeft, PanelBottom } from "lucide-react";

export function CommandPalette() {
    const open = useStore(uiStore, (s) => s.commandPaletteOpen);

    function close() {
        setCommandPaletteOpen(false);
    }

    function navigate(view: "analyze" | "queue" | "history" | "settings") {
        setView(view);
        close();
    }

    if (!open) return null;

    return (
        <ModalOverlay
            isOpen={open}
            onOpenChange={(o) => setCommandPaletteOpen(o)}
            isDismissable
            className="fixed inset-0 z-50 bg-black/60 flex items-start justify-center pt-[20vh]"
        >
            <Modal className="w-full max-w-lg mx-4 rounded-[8px] border border-[#2a2a3e] bg-[var(--surface-elevated)] shadow-2xl overflow-hidden">
                <Dialog className="outline-none">
                    {() => (
                        <Command className="bg-transparent">
                            <CommandInput placeholder="Type a command or search…" />
                            <CommandList>
                                <CommandEmpty>No results found.</CommandEmpty>

                                <CommandGroup heading="Navigation">
                                    <CommandItem onSelect={() => navigate("analyze")}>
                                        <Search className="w-4 h-4 mr-2 opacity-60" />
                                        <span>Analyze View</span>
                                        <span className="ml-auto kbd-chip">Ctrl+1</span>
                                    </CommandItem>
                                    <CommandItem onSelect={() => navigate("queue")}>
                                        <ArrowDownToLine className="w-4 h-4 mr-2 opacity-60" />
                                        <span>Queue View</span>
                                        <span className="ml-auto kbd-chip">Ctrl+2</span>
                                    </CommandItem>
                                    <CommandItem onSelect={() => navigate("history")}>
                                        <Clock className="w-4 h-4 mr-2 opacity-60" />
                                        <span>History View</span>
                                        <span className="ml-auto kbd-chip">Ctrl+3</span>
                                    </CommandItem>
                                    <CommandItem onSelect={() => navigate("settings")}>
                                        <Settings className="w-4 h-4 mr-2 opacity-60" />
                                        <span>Settings</span>
                                        <span className="ml-auto kbd-chip">Ctrl+4</span>
                                    </CommandItem>
                                </CommandGroup>

                                <CommandSeparator />

                                <CommandGroup heading="Actions">
                                    <CommandItem
                                        onSelect={() => {
                                            toggleNavPanel();
                                            close();
                                        }}
                                    >
                                        <PanelLeft className="w-4 h-4 mr-2 opacity-60" />
                                        <span>Toggle Sidebar</span>
                                        <span className="ml-auto kbd-chip">Ctrl+B</span>
                                    </CommandItem>
                                    <CommandItem
                                        onSelect={() => {
                                            toggleBottomDrawer();
                                            close();
                                        }}
                                    >
                                        <PanelBottom className="w-4 h-4 mr-2 opacity-60" />
                                        <span>Toggle Bottom Drawer</span>
                                        <span className="ml-auto kbd-chip">Ctrl+J</span>
                                    </CommandItem>
                                    <CommandItem
                                        onSelect={() => {
                                            window.dispatchEvent(new CustomEvent("rdlp-focus-search"));
                                            close();
                                        }}
                                    >
                                        <Search className="w-4 h-4 mr-2 opacity-60" />
                                        <span>Focus Command Bar</span>
                                        <span className="ml-auto kbd-chip">Ctrl+K</span>
                                    </CommandItem>
                                </CommandGroup>
                            </CommandList>
                        </Command>
                    )}
                </Dialog>
            </Modal>
        </ModalOverlay>
    );
}
