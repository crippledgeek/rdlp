// Shared normalization helpers for the format options panels.

import type { DownloadOptions } from "../../types";

/** Check whether any custom normalization parameters are set beyond a standard preset. */
export function isCustomNormalization(opts: DownloadOptions): boolean {
    if (!opts.normalizeAudio) return false;
    return (
        opts.loudnormTargetI !== null ||
        opts.loudnormTargetTp !== null ||
        opts.loudnormTargetLra !== null ||
        opts.loudnormDynamic === true ||
        opts.loudnormPrecompress === true ||
        opts.normalizeBoost === true
    );
}

/** Compute the Select value string for the normalization dropdown. */
export function getNormSelectValue(opts: DownloadOptions): string {
    if (isCustomNormalization(opts)) return "custom";
    if (opts.normalizeAudio === null) return "default";
    if (!opts.normalizeAudio) return "off";
    if (!opts.loudnorm) return "peak";
    if (opts.loudnormPreset === "broadcast") return "loudnorm-broadcast";
    if (opts.loudnormPreset === "loud") return "loudnorm-loud";
    if (
        opts.loudnormPreset === "streaming" ||
        opts.loudnormPreset === null
    )
        return "loudnorm-streaming";
    return "default";
}

/** Handle normalization Select value changes, returning the updated options. */
export function handleNormSelectChange(
    current: DownloadOptions,
    val: string,
): DownloadOptions {
    if (val === "default") {
        return {
            ...current,
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
        };
    }
    if (val === "off") {
        return {
            ...current,
            normalizeAudio: false,
            loudnorm: false,
            loudnormPreset: null,
            loudnormTargetI: null,
            loudnormTargetTp: null,
            loudnormTargetLra: null,
            loudnormDynamic: false,
            loudnormPrecompress: false,
            normalizeBoost: false,
            normalizeBoostDb: null,
        };
    }
    if (val === "peak") {
        return {
            ...current,
            normalizeAudio: true,
            loudnorm: false,
            loudnormPreset: null,
            loudnormTargetI: null,
            loudnormTargetTp: null,
            loudnormTargetLra: null,
            loudnormDynamic: null,
            loudnormPrecompress: null,
            normalizeBoost: null,
            normalizeBoostDb: null,
        };
    }
    if (val === "custom") {
        return {
            ...current,
            normalizeAudio: true,
            loudnorm: current.loudnorm ?? true,
        };
    }
    // Standard loudnorm preset — clear custom overrides
    const preset = val.replace("loudnorm-", "");
    return {
        ...current,
        normalizeAudio: true,
        loudnorm: true,
        loudnormPreset: preset,
        loudnormTargetI: null,
        loudnormTargetTp: null,
        loudnormTargetLra: null,
        loudnormDynamic: null,
        loudnormPrecompress: null,
        normalizeBoost: null,
        normalizeBoostDb: null,
    };
}
