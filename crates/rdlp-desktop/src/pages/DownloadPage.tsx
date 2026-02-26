import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowDownToLine, ArrowDown, Loader2 } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { JobCard } from "../components/JobCard";
import {
    downloadsQueryOptions,
    startDownload,
    cancelDownload,
    removeJob,
    buildDefaultOptions,
} from "../api/downloads";
import { settingsQueryOptions } from "../api/settings";
import { FormatDialog } from "../components/FormatDialog";
import type { DownloadOptions } from "../types";

const URL_PATTERN = /^https?:\/\/.+/i;

/** Download tab: paste a URL to quick-download or choose format. */
export function DownloadPage() {
    const [url, setUrl] = useState("");
    const [error, setError] = useState<string | null>(null);
    const [isStarting, setIsStarting] = useState(false);
    const [formatDialogUrl, setFormatDialogUrl] = useState<string | null>(null);
    const inputRef = useRef<HTMLInputElement>(null);

    const { data: settings = null } = useQuery(settingsQueryOptions());
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());

    // Listen for Ctrl+D focus event from App.tsx
    useEffect(() => {
        const handler = () => inputRef.current?.focus();
        window.addEventListener("rdlp-focus-url", handler);
        return () => window.removeEventListener("rdlp-focus-url", handler);
    }, []);

    const trimmedUrl = url.trim();
    const isValidUrl = URL_PATTERN.test(trimmedUrl);

    const handleQuickDownload = useCallback(async () => {
        if (!trimmedUrl) return;
        if (!isValidUrl) {
            setError("Please enter a valid URL starting with http:// or https://");
            return;
        }
        setError(null);
        setIsStarting(true);
        try {
            const options = buildDefaultOptions(settings);
            await startDownload(trimmedUrl, options);
            setUrl("");
        } catch (e) {
            setError("Failed to start download. The URL may be invalid or unsupported.");
        } finally {
            setIsStarting(false);
        }
    }, [trimmedUrl, isValidUrl, settings]);

    const handleKeyDown = useCallback(
        (e: React.KeyboardEvent) => {
            if (e.key === "Enter" && trimmedUrl && !isStarting) {
                e.preventDefault();
                void handleQuickDownload();
            }
        },
        [handleQuickDownload, trimmedUrl, isStarting],
    );

    const handleChooseFormat = useCallback(() => {
        if (!trimmedUrl) return;
        if (!isValidUrl) {
            setError("Please enter a valid URL starting with http:// or https://");
            return;
        }
        setError(null);
        setFormatDialogUrl(trimmedUrl);
    }, [trimmedUrl, isValidUrl]);

    const handleFormatConfirm = useCallback(
        (options: DownloadOptions, title?: string) => {
            if (!formatDialogUrl) return;
            void startDownload(formatDialogUrl, options, title)
                .then(() => {
                    setUrl("");
                    setFormatDialogUrl(null);
                })
                .catch(() => {
                    setError("Failed to start download.");
                });
        },
        [formatDialogUrl],
    );

    const handleCancel = (id: string) => {
        cancelDownload(id).catch((e) => console.error("Failed to cancel", e));
    };
    const handleRemove = (id: string) => {
        removeJob(id).catch((e) => console.error("Failed to remove", e));
    };
    const handleRetry = (retryUrl: string) => {
        const options = buildDefaultOptions(settings);
        startDownload(retryUrl, options).catch((e) =>
            console.error("Failed to retry", e),
        );
    };

    // Show last 5 jobs (most recent first)
    const recentJobs = [...jobs]
        .sort((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0))
        .slice(0, 5);

    return (
        <div className="max-w-2xl mx-auto">
            {/* Hero section */}
            <div className="flex flex-col items-center gap-4 pt-8 pb-6">
                <ArrowDownToLine className="h-10 w-10 text-muted-foreground opacity-50" aria-hidden="true" />
                <h2 className="text-lg font-semibold text-foreground">
                    Download a Video
                </h2>
            </div>

            {/* URL input row */}
            <div className="flex gap-2 mb-2">
                <Input
                    ref={inputRef}
                    type="url"
                    placeholder="Paste a video URL..."
                    value={url}
                    onChange={(e) => {
                        setUrl(e.target.value);
                        if (error) setError(null);
                    }}
                    onKeyDown={handleKeyDown}
                    className="flex-1 font-mono text-sm"
                    autoFocus
                />
                <Button
                    onClick={() => void handleQuickDownload()}
                    disabled={!trimmedUrl || isStarting}
                    className="shrink-0"
                    title={!trimmedUrl ? "Enter a URL first" : undefined}
                >
                    {isStarting ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                        <ArrowDown className="h-4 w-4" />
                    )}
                    <span className="ml-1.5">Download</span>
                </Button>
            </div>

            {/* Error message */}
            {error && (
                <Alert variant="destructive" className="mb-2">
                    <AlertDescription>{error}</AlertDescription>
                </Alert>
            )}

            {/* Choose Format button */}
            <div className="flex justify-center mb-4">
                <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleChooseFormat}
                    disabled={!trimmedUrl || isStarting}
                    className="text-muted-foreground text-xs"
                    title={!trimmedUrl ? "Enter a URL first" : undefined}
                >
                    Choose Format...
                </Button>
            </div>

            {/* Recent downloads */}
            {recentJobs.length > 0 && (
                <section className="mt-6">
                    <h3 className="section-heading">
                        Recent Downloads
                    </h3>
                    <div className="space-y-2">
                        {recentJobs.map((job) => (
                            <JobCard
                                key={job.id}
                                job={job}
                                onCancel={handleCancel}
                                onRemove={handleRemove}
                                onRetry={handleRetry}
                            />
                        ))}
                    </div>
                </section>
            )}

            {/* Format dialog (reused from search flow) */}
            {formatDialogUrl && (
                <FormatDialog
                    url={formatDialogUrl}
                    onConfirm={handleFormatConfirm}
                    onClose={() => setFormatDialogUrl(null)}
                />
            )}
        </div>
    );
}
