// SettingsNav: nav panel content for Settings view.
// Scrollspy using IntersectionObserver — highlights active section.

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

interface SectionDef {
    id: string;
    label: string;
}

const SECTIONS: SectionDef[] = [
    { id: "settings-general", label: "Search & Defaults" },
    { id: "settings-formats", label: "Format Defaults" },
    { id: "settings-output", label: "Output & Templates" },
    { id: "settings-postprocess", label: "Media & Embedding" },
    { id: "settings-normalization", label: "Audio Normalization" },
    { id: "settings-network", label: "Network" },
    { id: "settings-cookies", label: "Cookies" },
    { id: "settings-system", label: "System Info" },
];

export function SettingsNav() {
    const [activeId, setActiveId] = useState<string>(SECTIONS[0]?.id ?? "");
    const observerRef = useRef<IntersectionObserver | null>(null);

    useEffect(() => {
        // Use a threshold — when top of a section enters the upper 30% of viewport, mark it active
        observerRef.current = new IntersectionObserver(
            (entries) => {
                // Find the topmost intersecting section
                const visible = entries
                    .filter((e) => e.isIntersecting)
                    .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
                if (visible.length > 0) {
                    const first = visible[0];
                    if (first) setActiveId(first.target.id);
                }
            },
            {
                rootMargin: "-10% 0px -60% 0px",
                threshold: 0,
            },
        );

        SECTIONS.forEach(({ id }) => {
            const el = document.getElementById(id);
            if (el) observerRef.current!.observe(el);
        });

        return () => {
            observerRef.current?.disconnect();
        };
    }, []);

    const handleClick = (id: string) => {
        const el = document.getElementById(id);
        if (el) {
            el.scrollIntoView({ behavior: "smooth", block: "start" });
        }
    };

    return (
        <div className="flex flex-col h-full overflow-y-auto p-2 gap-0.5">
            <h2 className="section-heading px-2 mb-1">Settings</h2>
            {SECTIONS.map(({ id, label }) => {
                const isActive = activeId === id;
                return (
                    <button
                        key={id}
                        onClick={() => handleClick(id)}
                        className={cn(
                            "text-left px-3 py-1.5 rounded-[4px] text-[12px] transition-colors",
                            isActive
                                ? "bg-[#1a2a4a] text-[#4a9eff]"
                                : "text-[#aaaaaa] hover:bg-[#141428] hover:text-[#eeeeee]",
                        )}
                    >
                        {label}
                    </button>
                );
            })}
        </div>
    );
}
