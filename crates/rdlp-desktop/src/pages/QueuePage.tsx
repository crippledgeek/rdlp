import { useQuery } from "@tanstack/react-query";
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
        void cancelDownload(id);
    };

    const handleRemove = (id: string) => {
        void removeJob(id);
    };

    const handleRetry = (url: string) => {
        const options = buildDefaultOptions(settings);
        void startDownload(url, options);
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
                <div className="status-message">
                    <svg className="status-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                        <polyline points="7 10 12 15 17 10" />
                        <line x1="12" y1="15" x2="12" y2="3" />
                    </svg>
                    <p>No downloads yet.</p>
                </div>
            </div>
        );
    }

    return (
        <div className="queue-page">
            {activeJobs.length > 0 && (
                <section className="queue-section">
                    <h3 className="queue-section-title">
                        Active ({activeJobs.length})
                    </h3>
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
                    <h3 className="queue-section-title">
                        History ({completedJobs.length})
                    </h3>
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
