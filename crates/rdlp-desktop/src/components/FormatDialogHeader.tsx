// Context header for the format dialog showing thumbnail and title.

import {
    DialogDescription,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";

interface FormatDialogHeaderProps {
    title: string | undefined;
    thumbnailUrl: string | null | undefined;
}

/** Zone 1: Thumbnail + title inside the dialog header. */
export function FormatDialogHeader({ title, thumbnailUrl }: FormatDialogHeaderProps) {
    return (
        <DialogHeader className="px-5 pt-4 pb-3 border-b border-border shrink-0">
            <div className="flex items-center gap-3">
                {thumbnailUrl && (
                    <img
                        src={thumbnailUrl}
                        alt=""
                        className="w-12 h-[27px] object-cover rounded-sm shrink-0 bg-muted"
                    />
                )}
                <div className="min-w-0 flex-1">
                    <DialogTitle className="text-sm font-semibold text-foreground truncate pr-8">
                        {title ?? "Choose Format"}
                    </DialogTitle>
                    <DialogDescription className="sr-only">
                        Select a format and download options for this video
                    </DialogDescription>
                </div>
            </div>
        </DialogHeader>
    );
}
