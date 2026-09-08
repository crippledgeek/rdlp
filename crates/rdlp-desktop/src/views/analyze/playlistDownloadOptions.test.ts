// @vitest-environment node
// The playlist batch path hardcoded `embedThumbnail: true`, so turning the
// setting off in Settings still embedded thumbnails on every episode of every
// playlist download. `embedThumbnail` is a bare `bool` in the Rust
// `DownloadOptions` and is consumed unconditionally, so the merge has to
// happen here — the single-download path in DownloadConfig.tsx already does it.

import { describe, test, expect } from "vitest";
import { playlistDownloadOptions } from "./playlistDownloadOptions";
import type { AppSettings } from "@/types";

function settingsWith(embedThumbnail: boolean) {
    return { embed_thumbnail: embedThumbnail } as unknown as AppSettings;
}

describe("playlistDownloadOptions — embedThumbnail", () => {
    test("honours an embed_thumbnail=false setting", () => {
        expect(playlistDownloadOptions(settingsWith(false)).embedThumbnail).toBe(false);
    });

    test("honours an embed_thumbnail=true setting", () => {
        expect(playlistDownloadOptions(settingsWith(true)).embedThumbnail).toBe(true);
    });

    // Matches DownloadConfig.tsx's `settings?.embed_thumbnail ?? true`: before
    // the settings query resolves there is no value to honour, and embedding
    // is the product default.
    test("defaults to true when settings have not loaded", () => {
        expect(playlistDownloadOptions(undefined).embedThumbnail).toBe(true);
    });

    // The rest of the batch options stay "unspecified" so the backend applies
    // its own settings layer — including the subtitle-language merge this
    // commit adds.
    test("leaves the backend-resolved fields unspecified", () => {
        const options = playlistDownloadOptions(settingsWith(true));
        expect(options.subtitleLangs).toBeNull();
        expect(options.format).toBeNull();
        expect(options.outputDir).toBeNull();
    });
});
