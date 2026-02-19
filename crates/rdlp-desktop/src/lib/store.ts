// Zustand stores for search, download queue, and settings state management.
//
// Three stores:
//   useSearchStore   - search query, filters, providers, results
//   useQueueStore    - download jobs, format selection, optimistic updates
//   useSettingsStore - application settings, directory picker

import { create } from "zustand";
import type {
    AppSettings,
    DownloadJob,
    DownloadOptions,
    FormatListResponse,
    SearchFilter,
    SearchFilterDescriptor,
    SearchResultPreview,
    SearchSiteInfo,
} from "../types";
import * as api from "./tauri";

// ========== Search Store ==========

type SearchStatus = "idle" | "loading" | "results" | "empty" | "error";

interface SearchState {
    status: SearchStatus;
    query: string;
    site: string;
    filters: SearchFilter[];
    results: SearchResultPreview[];
    providers: SearchSiteInfo[];
    filterDescriptors: SearchFilterDescriptor[];
    error: string | null;

    setQuery: (query: string) => void;
    setSite: (site: string) => void;
    setFilters: (filters: SearchFilter[]) => void;
    loadProviders: () => Promise<void>;
    loadFilters: (site: string) => Promise<void>;
    search: () => Promise<void>;
    clear: () => void;
}

export const useSearchStore = create<SearchState>()((set, get) => ({
    status: "idle",
    query: "",
    site: "",
    filters: [],
    results: [],
    providers: [],
    filterDescriptors: [],
    error: null,

    setQuery: (query) => set({ query }),

    setSite: (site) => set({ site }),

    setFilters: (filters) => set({ filters }),

    loadProviders: async () => {
        try {
            const providers = await api.getSearchProviders();
            const state = get();
            const updates: Partial<SearchState> = { providers };
            if (state.site === "" && providers.length > 0) {
                updates.site = providers[0].name;
            }
            set(updates);
        } catch (err) {
            set({
                error: err instanceof Error ? err.message : String(err),
                status: "error",
            });
        }
    },

    loadFilters: async (site) => {
        try {
            const filterDescriptors = await api.getSearchFilters(site);
            set({ filterDescriptors });
        } catch (err) {
            set({
                error: err instanceof Error ? err.message : String(err),
                status: "error",
            });
        }
    },

    search: async () => {
        const { query, site } = get();
        if (query.trim() === "" || site === "") {
            return;
        }

        set({ status: "loading", error: null });

        try {
            const { filters } = get();
            const response = await api.searchContent(query, site, filters);

            if (response.results.length === 0) {
                set({ status: "empty", results: [] });
            } else {
                set({ status: "results", results: response.results });
            }
        } catch (err) {
            set({
                status: "error",
                error: err instanceof Error ? err.message : String(err),
            });
        }
    },

    clear: () =>
        set({
            status: "idle",
            query: "",
            filters: [],
            results: [],
            error: null,
        }),
}));

// ========== Queue Store ==========

const DEFAULT_DOWNLOAD_OPTIONS: DownloadOptions = {
    format: null,
    outputDir: null,
    subtitles: false,
    subtitleLangs: [],
    remux: null,
    extractAudio: null,
    embedThumbnail: true,
};

interface QueueState {
    jobs: DownloadJob[];
    selectedFormat: FormatListResponse | null;

    refreshQueue: () => Promise<void>;
    startDownload: (url: string) => Promise<string>;
    cancelDownload: (jobId: string) => Promise<void>;
    removeJob: (jobId: string) => Promise<void>;
    updateJobFromProgress: (
        jobId: string,
        progress: number,
        speed: string | null,
        eta: string | null,
    ) => void;
    markJobCompleted: (jobId: string, filepath: string) => void;
    markJobFailed: (jobId: string, error: string, retryable: boolean) => void;
    loadFormats: (url: string) => Promise<FormatListResponse>;
    clearFormats: () => void;
}

export const useQueueStore = create<QueueState>()((set, get) => ({
    jobs: [],
    selectedFormat: null,

    refreshQueue: async () => {
        const jobs = await api.getQueue();
        set({ jobs });
    },

    startDownload: async (url) => {
        const jobId = await api.startDownload(url, DEFAULT_DOWNLOAD_OPTIONS);
        await get().refreshQueue();
        return jobId;
    },

    cancelDownload: async (jobId) => {
        await api.cancelDownload(jobId);
        await get().refreshQueue();
    },

    removeJob: async (jobId) => {
        await api.removeJob(jobId);
        await get().refreshQueue();
    },

    updateJobFromProgress: (jobId, progress, speed, eta) =>
        set((state) => ({
            jobs: state.jobs.map((job) =>
                job.id === jobId ? { ...job, progress, speed, eta } : job,
            ),
        })),

    markJobCompleted: (jobId, filepath) =>
        set((state) => ({
            jobs: state.jobs.map((job) =>
                job.id === jobId
                    ? {
                          ...job,
                          status: "completed" as const,
                          progress: 1.0,
                          output_path: filepath,
                          completed_at: Math.floor(Date.now() / 1000),
                      }
                    : job,
            ),
        })),

    markJobFailed: (jobId, error, retryable) =>
        set((state) => ({
            jobs: state.jobs.map((job) =>
                job.id === jobId
                    ? {
                          ...job,
                          status: "failed" as const,
                          error,
                          retryable,
                      }
                    : job,
            ),
        })),

    loadFormats: async (url) => {
        const formats = await api.getFormats(url);
        set({ selectedFormat: formats });
        return formats;
    },

    clearFormats: () => set({ selectedFormat: null }),
}));

// ========== Settings Store ==========

interface SettingsState {
    settings: AppSettings | null;
    loading: boolean;

    loadSettings: () => Promise<void>;
    updateSettings: (settings: AppSettings) => Promise<void>;
    pickDirectory: () => Promise<string | null>;
}

export const useSettingsStore = create<SettingsState>()((set) => ({
    settings: null,
    loading: false,

    loadSettings: async () => {
        set({ loading: true });
        try {
            const settings = await api.getSettings();
            set({ settings, loading: false });
        } catch {
            set({ loading: false });
        }
    },

    updateSettings: async (settings) => {
        await api.updateSettings(settings);
        set({ settings });
    },

    pickDirectory: async () => {
        const result = await api.pickDirectory();
        return result;
    },
}));
