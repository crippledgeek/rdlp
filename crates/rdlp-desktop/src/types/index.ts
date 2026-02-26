// TypeScript interfaces mirroring the Rust types from rdlp-desktop src-tauri.
//
// Serialization conventions:
//   - Rust structs without #[serde(rename_all)] use snake_case field names.
//   - Rust structs with #[serde(rename_all = "camelCase")] use camelCase.
//   - Rust Option<T> maps to T | null.
//   - Rust enums with #[serde(rename_all = "lowercase")] become string literals.

// ========== Search Types ==========

/** A site that supports search (snake_case — default serde). */
export interface SearchSiteInfo {
    name: string;
    display_name: string;
}

/** A concrete filter key-value applied to a search. */
export interface SearchFilter {
    key: string;
    value: string;
}

/** Describes a filter a site supports (for UI construction). */
export interface SearchFilterDescriptor {
    key: string;
    display_name: string;
    allowed_values: SearchFilterValue[];
    default: string | null;
}

/** A single selectable value inside a filter descriptor. */
export interface SearchFilterValue {
    value: string;
    label: string;
}

/** A lightweight preview of a single search result. */
export interface SearchResultPreview {
    video_url: string;
    title: string;
    thumbnail_url: string | null;
    duration: number | null;
    view_count: number | null;
    upload_date: string | null;
}

/** Response for a single search page with pagination metadata. */
export interface SearchPageResponse {
    results: SearchResultPreview[];
    page: number;
    has_more: boolean;
    total_estimate: number | null;
}

// ========== Format Types ==========

/** Frontend-facing format information (snake_case — default serde). */
export interface FormatInfo {
    format_id: string;
    ext: string;
    format_note: string | null;
    width: number | null;
    height: number | null;
    fps: number | null;
    tbr: number | null;
    vcodec: string | null;
    acodec: string | null;
    filesize: number | null;
    vbr: number | null;
    abr: number | null;
    asr: number | null;
    protocol: string;
    has_video: boolean;
    has_audio: boolean;
}

/** Subtitle availability for a single language. */
export interface SubtitleInfo {
    lang: string;
    formats: string[];
}

/** Complete response for the get_formats command. */
export interface FormatListResponse {
    title: string;
    formats: FormatInfo[];
    subtitles: SubtitleInfo[];
    thumbnail_url: string | null;
    duration: number | null;
}

// ========== Download Types ==========

/** Lifecycle status of a download job (#[serde(rename_all = "lowercase")]). */
export type JobStatus =
    | "pending"
    | "running"
    | "completed"
    | "failed"
    | "cancelled";

/** Serializable metadata for a single download job (snake_case — default serde). */
export interface DownloadJob {
    id: string;
    url: string;
    title: string | null;
    status: JobStatus;
    progress: number | null;
    speed: string | null;
    eta: string | null;
    error: string | null;
    retryable: boolean;
    started_at: number | null;
    completed_at: number | null;
    output_path: string | null;
    /** Original options used to start this job (populated by backend). */
    options: DownloadOptions | null;
    /** Latest log message from the download-log event (frontend-only, not from Rust). */
    statusMessage: string | null;
    /** Accumulated log messages from download-log events (frontend-only, not from Rust). */
    logMessages?: string[];
}

/**
 * Container formats matching Rust `ContainerFormat` (#[serde(rename_all = "lowercase")]).
 */
export type ContainerFormat =
    | "mp4" | "mkv" | "webm" | "mov" | "ts" | "flv" | "avi"
    | "threegp" | "mpg" | "f4v" | "asf" | "mxf" | "vob" | "dv"
    | "nut" | "ivf" | "ogg" | "m4a" | "mp3" | "wav" | "flac"
    | "opus" | "aac" | "aiff" | "mka" | "wv" | "caf" | "ac3";

/**
 * Audio formats matching Rust `AudioFormat` (#[serde(rename_all = "lowercase")]).
 */
export type AudioFormat =
    | "mp3" | "aac" | "m4a" | "opus" | "vorbis" | "flac" | "alac"
    | "wav" | "ac3" | "eac3" | "dts" | "mp2" | "wavpack" | "tta";

/**
 * Subtitle formats matching Rust `SubtitleFormat` (#[serde(rename_all = "lowercase")]).
 */
export type SubtitleFormat = "srt" | "vtt" | "ass" | "ssa" | "lrc";

/**
 * Frontend-supplied download options.
 *
 * Uses camelCase because the Rust struct has #[serde(rename_all = "camelCase")].
 */
export interface DownloadOptions {
    format: string | null;
    outputDir: string | null;
    subtitles: boolean;
    subtitleLangs: string[];
    remux: ContainerFormat | null;
    extractAudio: AudioFormat | null;
    embedThumbnail: boolean;
    audioMultistreams: boolean;
    recodeVideo: ContainerFormat | null;
    normalizeAudio: boolean | null;
    loudnorm: boolean | null;
    loudnormPreset: string | null;
    loudnormTargetI: number | null;
    loudnormTargetTp: number | null;
    loudnormTargetLra: number | null;
    loudnormDynamic: boolean | null;
    loudnormPrecompress: boolean | null;
    normalizeBoost: boolean | null;
    normalizeBoostDb: number | null;
    embedSubtitles: boolean | null;
}

// ========== Event Payloads (camelCase — Rust uses #[serde(rename_all = "camelCase")]) ==========

/** Download progress payload emitted as "download-progress". */
export interface DownloadProgressPayload {
    jobId: string;
    progress: number;
    speed: string | null;
    eta: string | null;
    downloadedBytes: number;
    totalBytes: number | null;
}

/** Download completion payload emitted as "download-complete". */
export interface DownloadCompletePayload {
    jobId: string;
    filepath: string;
}

/** Download error payload emitted as "download-error". */
export interface DownloadErrorPayload {
    jobId: string;
    error: string;
    retryable: boolean;
}

/** Format-selected payload emitted as "format-selected". */
export interface FormatSelectedPayload {
    jobId: string;
    formatId: string;
    quality: string;
}

/** Log message payload emitted as "download-log". */
export interface DownloadLogPayload {
    jobId: string;
    level: "info" | "warn" | "debug";
    message: string;
}

// ========== Settings Types ==========

/**
 * Application settings that persist between sessions (snake_case — default serde).
 *
 * Rust PathBuf serializes as a string.
 */
export interface AppSettings {
    output_dir: string;
    default_remux: ContainerFormat | null;
    default_extract_audio: AudioFormat | null;
    default_subtitle_format: SubtitleFormat | null;
    default_subtitle_langs: string[];
    embed_thumbnail: boolean;
    write_thumbnail: boolean;
    embed_metadata: boolean;
    verbose: boolean;
    default_search_provider: string | null;
    output_template: string | null;
    cookies_from_browser: string | null;
    cookies_file: string | null;
    proxy: string | null;
    rate_limit: string | null;
    normalize_audio: boolean;
    audio_gain_target: number | null;
    loudnorm: boolean;
    loudnorm_preset: string | null;
    loudnorm_target_i: number | null;
    loudnorm_target_tp: number | null;
    loudnorm_target_lra: number | null;
    loudnorm_dynamic: boolean;
    loudnorm_precompress: boolean;
    normalize_boost: boolean;
    normalize_boost_db: number | null;
    embed_subtitles: boolean;
}

// ========== Error Types ==========

/**
 * Frontend-facing error from Tauri IPC commands.
 *
 * Rust uses #[serde(tag = "kind", content = "data")] so the JSON shape is:
 *   { kind: "NetworkError", data: { message: "...", retryable: true } }
 */
export interface AppError {
    kind: string;
    data: Record<string, unknown>;
}

/** View mode for search results display. */
export type ViewMode = "list" | "grid";
