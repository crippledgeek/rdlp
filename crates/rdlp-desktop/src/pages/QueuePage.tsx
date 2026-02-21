import { useQuery } from "@tanstack/react-query";
import { Download } from "lucide-react";
import { JobCard } from "../components/JobCard";
import {
    downloadsQueryOptions,
    cancelDownload,
    removeJob,
    startDownload,
    buildDefaultOptions,
} from "../api/downloads";
import { settingsQueryOptions } from "../api/settings";

/** Download queue page showing active and completed jobs. */
export function QueuePage() {
    const { data: jobs = [] } = useQuery(downloadsQueryOptions());
    const { data: settings = null } = useQuery(settingsQueryOptions());

    const handleCancel = (id: string) => {
        cancelDownload(id).catch((e) =>
            console.error("Failed to cancel download", e),
        );
    };

    const handleRemove = (id: string) => {
        removeJob(id).catch((e) =>
            console.error("Failed to remove job", e),
        );
    };

    const handleRetry = (url: string) => {
        const options = buildDefaultOptions(settings);
        startDownload(url, options).catch((e) =>
            console.error("Failed to retry download", e),
        );
    };

    const activeJobs = jobs.filter(
        (j) => j.status === "pending" || j.status === "running",
    );
    const completedJobs = jobs.filter(
        (j) =>
            j.status === "completed" ||
            j.status === "failed" ||
            j.status === "cancelled",
    );

    if (jobs.length === 0) {
        return (
            <div className="max-w-3xl">
                <div
                    className="flex flex-col items-center justify-center
                        gap-3 py-16 text-muted-foreground"
                >
                    <Download className="h-10 w-10 opacity-40" />
                    <p>No downloads yet.</p>
                </div>
            </div>
        );
    }

    return (
        <div className="max-w-3xl">
            {activeJobs.length > 0 && (
                <section className="mb-6">
                    <h3
                        className="mb-2.5 border-b border-border pb-1.5
                            text-[11px] font-bold uppercase tracking-widest
                            text-muted-foreground"
                    >
                        Active ({activeJobs.length})
                    </h3>
                    <div className="space-y-2">
                        {activeJobs.map((job) => (
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

            {completedJobs.length > 0 && (
                <section className="mb-6">
                    <h3
                        className="mb-2.5 border-b border-border pb-1.5
                            text-[11px] font-bold uppercase tracking-widest
                            text-muted-foreground"
                    >
                        History ({completedJobs.length})
                    </h3>
                    <div className="space-y-2">
                        {completedJobs.map((job) => (
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
        </div>
    );
}
