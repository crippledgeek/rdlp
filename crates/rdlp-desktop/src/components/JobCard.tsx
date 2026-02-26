import { useState } from "react";
import type { DownloadJob } from "../types";
import { cn } from "@/lib/utils";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { invokeTyped } from "../api/invokeClient";

interface JobCardProps {
    job: DownloadJob;
    onCancel: (id: string) => void;
    onRemove: (id: string) => void;
    onRetry: (job: DownloadJob) => void;
}

/** Format a progress fraction in [0.0, 1.0] as a percentage string. */
function formatPercent(progress: number | null): string {
    if (progress === null) return "0%";
    const pct = progress * 100;
    return `${pct.toFixed(1)}%`;
}

/** Return Tailwind classes for a job status badge. */
function statusClasses(status: string): string {
    switch (status) {
        case "pending":
            return "bg-yellow-500/10 text-yellow-500 border-yellow-500/20";
        case "running":
            return "bg-primary/10 text-primary border-primary/20";
        case "completed":
            return "bg-primary/10 text-primary border-primary/20";
        case "failed":
            return "bg-destructive/10 text-destructive border-destructive/20";
        case "cancelled":
            return "bg-gray-500/10 text-gray-500 border-gray-500/20";
        default:
            return "";
    }
}

/** Displays a single download job with status, progress, and action buttons. */
export function JobCard({ job, onCancel, onRemove, onRetry }: JobCardProps) {
    const [revealError, setRevealError] = useState<string | null>(null);
    const displayTitle = job.title ?? job.url;
    const isRunning = job.status === "running";
    const isFailed = job.status === "failed";
    const isTerminal =
        job.status === "completed" ||
        job.status === "failed" ||
        job.status === "cancelled";

    return (
        <Card className="gap-0 rounded-lg border-border py-0">
            <CardContent className="space-y-2.5 p-3">
                <div className="flex items-center justify-between gap-2">
                    <span
                        className="min-w-0 flex-1 truncate text-sm
                            font-medium text-foreground"
                        title={job.url}
                    >
                        {displayTitle}
                    </span>
                    <Badge
                        variant="outline"
                        className={cn(
                            "text-[10px] capitalize",
                            statusClasses(job.status),
                        )}
                    >
                        {job.status}
                    </Badge>
                </div>

                {isRunning && (
                    <div className="space-y-1.5">
                        <Progress
                            value={(job.progress ?? 0) * 100}
                            className="h-1"
                        />
                        <div
                            className="flex items-center gap-3 text-xs
                                text-muted-foreground"
                        >
                            <span className="font-medium text-foreground">
                                {formatPercent(job.progress)}
                            </span>
                            {job.speed && <span>{job.speed}</span>}
                            {job.eta && <span>ETA: {job.eta}</span>}
                        </div>
                        {job.statusMessage && (
                            <p
                                aria-live="polite"
                                className="truncate text-xs text-muted-foreground"
                            >
                                {job.statusMessage}
                            </p>
                        )}
                    </div>
                )}

                {isFailed && job.error && (
                    <Alert variant="destructive" className="py-2">
                        <AlertDescription className="text-xs">
                            {job.error}
                        </AlertDescription>
                    </Alert>
                )}

                {revealError && (
                    <Alert variant="destructive" className="py-2">
                        <AlertDescription className="text-xs">
                            Could not reveal file: {revealError}
                        </AlertDescription>
                    </Alert>
                )}

                <div className="flex items-center gap-2">
                    {isRunning && (
                        <Button
                            variant="outline"
                            size="sm"
                            className="hover:border-destructive/30
                                hover:bg-destructive/10
                                hover:text-destructive"
                            onClick={() => onCancel(job.id)}
                        >
                            Cancel
                        </Button>
                    )}
                    {isFailed && job.retryable && (
                        <Button
                            variant="outline"
                            size="sm"
                            className="hover:border-primary/30
                                hover:bg-primary/10 hover:text-primary"
                            onClick={() => onRetry(job)}
                        >
                            Retry
                        </Button>
                    )}
                    {job.status === "completed" && job.output_path && (
                        <Button
                            variant="outline"
                            size="sm"
                            onClick={() => {
                                setRevealError(null);
                                invokeTyped<void>("reveal_in_folder", {
                                    path: job.output_path,
                                }).catch((e) => {
                                    const msg =
                                        e && typeof e === "object" && "message" in e
                                            ? String(e.message)
                                            : String(e);
                                    setRevealError(msg);
                                });
                            }}
                        >
                            Reveal in Folder
                        </Button>
                    )}
                    {isTerminal && (
                        <Button
                            variant="outline"
                            size="sm"
                            onClick={() => onRemove(job.id)}
                        >
                            Remove
                        </Button>
                    )}
                </div>
            </CardContent>
        </Card>
    );
}
