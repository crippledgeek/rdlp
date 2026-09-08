// Default download options for a playlist batch.
//
// Extracted from EpisodeList so the settings merge has a testable seam, the
// same role `build_subtitle_options` plays on the Rust side.
//
// Most fields are `null` = "unspecified", which the backend resolves against
// AppSettings. `embedThumbnail` cannot be: it is a bare `bool` in the Rust
// `DownloadOptions` and `commands/download.rs` consumes it unconditionally, so
// whatever is sent here IS the final value and the settings default must be
// applied on this side — exactly as `DownloadConfig.tsx` does for the
// single-download path.

import { DEFAULT_EMBED_THUMBNAIL } from "@/lib/settingsDefaults";
import type { AppSettings, DownloadOptions } from "@/types";

export function playlistDownloadOptions(settings: AppSettings | undefined): DownloadOptions {
    return {
        format: null,
        outputDir: null,
        subtitles: false,
        subtitleLangs: null,
        remux: null,
        extractAudio: null,
        embedThumbnail: settings?.embed_thumbnail ?? DEFAULT_EMBED_THUMBNAIL,
        audioMultistreams: false,
        recodeVideo: null,
        normalizeAudio: null,
        loudnorm: null,
        loudnormPreset: null,
        loudnormTargetI: null,
        loudnormTargetTp: null,
        loudnormTargetLra: null,
        loudnormDynamic: null,
        loudnormPrecompress: null,
        normalizeBoost: null,
        normalizeBoostDb: null,
        embedSubtitles: null,
        videoEncoder: null,
        recodeAudio: null,
        recodeContainer: null,
        recodeThreads: null,
        recodePreset: null,
        recodeDeadline: null,
        recodeCpuUsed: null,
        recodeSpeedLevel: null,
        verbose: null,
    };
}
