// @vitest-environment node
// Unit tests for formatUtils.ts — formatBrief, formatSize, formatBitrate,
// formatResolution, formatType.

import { describe, test, expect } from "vitest";
import type { FormatInfo } from "../../types";
import { formatBrief, formatSize, formatBitrate, formatResolution, formatType } from "./formatUtils";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeFormat(overrides: Partial<FormatInfo> = {}): FormatInfo {
    return {
        format_id: "123",
        ext: "mp4",
        format_note: null,
        width: null,
        height: null,
        fps: null,
        tbr: null,
        vcodec: null,
        acodec: null,
        filesize: null,
        vbr: null,
        abr: null,
        asr: null,
        protocol: "https",
        has_video: true,
        has_audio: true,
        audio_group_id: null,
        ...overrides,
    };
}

// ---------------------------------------------------------------------------
// formatBrief
// ---------------------------------------------------------------------------

describe("formatBrief", () => {
    test("uses height when present", () => {
        expect(formatBrief(makeFormat({ height: 1080 }))).toBe("1080p mp4");
    });

    test("falls back to format_note when no height", () => {
        expect(formatBrief(makeFormat({ format_note: "HQ", height: null }))).toBe("HQ mp4");
    });

    test("returns just ext when neither height nor note", () => {
        expect(formatBrief(makeFormat({ height: null, format_note: null }))).toBe("mp4");
    });

    test("prefers height over format_note", () => {
        expect(formatBrief(makeFormat({ height: 720, format_note: "HD" }))).toBe("720p mp4");
    });
});

// ---------------------------------------------------------------------------
// formatSize
// ---------------------------------------------------------------------------

describe("formatSize", () => {
    test("returns em-dash for null", () => {
        expect(formatSize(null)).toBe("\u2014");
    });

    test("formats bytes below 1024 as B", () => {
        expect(formatSize(512)).toBe("512 B");
    });

    test("formats kilobytes", () => {
        expect(formatSize(2048)).toBe("2 KB");
    });

    test("formats megabytes", () => {
        expect(formatSize(1_048_576)).toBe("1.0 MB");
    });

    test("formats gigabytes", () => {
        expect(formatSize(1_073_741_824)).toBe("1.0 GB");
    });

    test("formats 0 bytes", () => {
        expect(formatSize(0)).toBe("0 B");
    });
});

// ---------------------------------------------------------------------------
// formatBitrate
// ---------------------------------------------------------------------------

describe("formatBitrate", () => {
    test("returns em-dash for null", () => {
        expect(formatBitrate(null)).toBe("\u2014");
    });

    test("formats sub-1000 kbps as kbps", () => {
        expect(formatBitrate(500)).toBe("500 kbps");
    });

    test("formats >= 1000 kbps as Mbps", () => {
        expect(formatBitrate(2500)).toBe("2.5 Mbps");
    });

    test("formats exactly 1000 kbps as Mbps", () => {
        expect(formatBitrate(1000)).toBe("1.0 Mbps");
    });
});

// ---------------------------------------------------------------------------
// formatResolution
// ---------------------------------------------------------------------------

describe("formatResolution", () => {
    test("returns WxH when both width and height present", () => {
        expect(formatResolution(makeFormat({ width: 1920, height: 1080 }))).toBe("1920\u00d71080");
    });

    test("returns height-only when width missing", () => {
        expect(formatResolution(makeFormat({ width: null, height: 720 }))).toBe("720p");
    });

    test("returns format_note when neither dimension is present", () => {
        expect(formatResolution(makeFormat({ width: null, height: null, format_note: "audio only" }))).toBe("audio only");
    });

    test("returns em-dash when no dimension and no note", () => {
        expect(formatResolution(makeFormat({ width: null, height: null, format_note: null }))).toBe("\u2014");
    });
});

// ---------------------------------------------------------------------------
// formatType
// ---------------------------------------------------------------------------

describe("formatType", () => {
    test("returns HLS label for m3u8 protocol", () => {
        const result = formatType(makeFormat({ protocol: "m3u8" }));
        expect(result.label).toBe("HLS");
    });

    test("returns V+A label for video+audio stream", () => {
        const result = formatType(makeFormat({ protocol: "https", has_video: true, has_audio: true }));
        expect(result.label).toBe("V+A");
    });

    test("returns Video label for video-only stream", () => {
        const result = formatType(makeFormat({ protocol: "https", has_video: true, has_audio: false }));
        expect(result.label).toBe("Video");
    });

    test("returns Audio label for audio-only stream", () => {
        const result = formatType(makeFormat({ protocol: "https", has_video: false, has_audio: false }));
        expect(result.label).toBe("Audio");
    });

    test("HLS check takes priority over stream flags", () => {
        const result = formatType(makeFormat({ protocol: "m3u8_native", has_video: true, has_audio: true }));
        expect(result.label).toBe("HLS");
    });

    test("returns className strings", () => {
        const result = formatType(makeFormat({ protocol: "https", has_video: true, has_audio: true }));
        expect(typeof result.className).toBe("string");
        expect(result.className.length).toBeGreaterThan(0);
    });
});
