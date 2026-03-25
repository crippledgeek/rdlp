// EmptyState: shown on cold start before any URL is entered.
// Contains a dropzone hint with keyboard shortcut.

import { Link2 } from "lucide-react";

export function EmptyState() {
    return (
        <div className="flex flex-col items-center justify-center h-full gap-6 p-8 select-none">
            <div className="flex flex-col items-center gap-3 text-center">
                <div className="w-12 h-12 rounded-full bg-[#0e0e1e] border border-[#1a1a2e] flex items-center justify-center">
                    <Link2 className="w-5 h-5 text-[#4a9eff]" />
                </div>
                <div>
                    <p className="text-[15px] font-medium text-[#eeeeee] mb-1">Paste a URL to begin</p>
                    <p className="text-[12px] text-[#666666]">
                        Supports video sites with HLS, HTTP, and DASH streams
                    </p>
                </div>
            </div>

            <div className="flex items-center gap-2 text-[11px] text-[#444444]">
                <span className="kbd-chip">Ctrl+K</span>
                <span>to focus the command bar</span>
            </div>
        </div>
    );
}
