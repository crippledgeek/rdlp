// Shared format option arrays and sentinel value used across
// FormatOptionsPanel, DownloadOptionsPanel, and SettingsPage sections.

import type { AudioFormat, ContainerFormat } from "../../types";

/** Sentinel value for "no selection" since Radix Select does not support empty string values. */
export const NONE_SENTINEL = "none";

/** Common remux options shown in the UI. */
export const REMUX_OPTIONS: ReadonlyArray<{ value: ContainerFormat; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
];

/** Common recode (transcode) options shown in the UI. */
export const RECODE_OPTIONS: ReadonlyArray<{ value: ContainerFormat; label: string }> = [
    { value: "mp4", label: "MP4" },
    { value: "mkv", label: "MKV" },
    { value: "webm", label: "WebM" },
];

/** Common audio extraction options shown in the UI. */
export const AUDIO_OPTIONS: ReadonlyArray<{ value: AudioFormat; label: string }> = [
    { value: "mp3", label: "MP3" },
    { value: "aac", label: "AAC" },
    { value: "opus", label: "Opus" },
    { value: "flac", label: "FLAC" },
];
