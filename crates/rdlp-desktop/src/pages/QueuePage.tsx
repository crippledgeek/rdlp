import { useEffect } from "react";
import { useQueueStore } from "../lib/store";
import { JobCard } from "../components/JobCard";

/** Download queue page showing active and completed jobs. */
export function QueuePage() {
    const jobs = useQueueStore((s) => s.jobs);
    const refreshQueue = useQueueStore((s) => s.refreshQueue);
    const cancelDownload = useQueueStore((s) => s.cancelDownload);
    const removeJob = useQueueStore((s) => s.removeJob);
    const startDownload = useQueueStore((s) => s.startDownload);

    useEffect(() => {
        void refreshQueue();
    }, [refreshQueue]);

    const handleCancel = (id: string) => {
        void cancelDownload(id);
    };

    const handleRemove = (id: string) => {
        void removeJob(id);
    };

    const handleRetry = (url: string) => {
        void startDownload(url);
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
            <div className="queue-page">
                <p className="status-message">No downloads yet.</p>
            </div>
        );
    }

    return (
        <div className="queue-page">
            {activeJobs.length > 0 && (
                <section className="queue-section">
                    <h3 className="queue-section-title">Active</h3>
                    {activeJobs.map((job) => (
                        <JobCard
                            key={job.id}
                            job={job}
                            onCancel={handleCancel}
                            onRemove={handleRemove}
                            onRetry={handleRetry}
                        />
                    ))}
                </section>
            )}

            {completedJobs.length > 0 && (
                <section className="queue-section">
                    <h3 className="queue-section-title">Completed</h3>
                    {completedJobs.map((job) => (
                        <JobCard
                            key={job.id}
                            job={job}
                            onCancel={handleCancel}
                            onRemove={handleRemove}
                            onRetry={handleRetry}
                        />
                    ))}
                </section>
            )}
        </div>
    );
}
