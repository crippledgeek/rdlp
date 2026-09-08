// Pins the call site the #589 thumbnail defect actually lived at.
//
// `playlistDownloadOptions.test.ts` proves the helper merges settings; it
// cannot prove EpisodeList *passes* them. Changing the call to
// `playlistDownloadOptions(undefined)` reinstates the bug in full and leaves
// every helper test green — so this test drives the real button and asserts on
// what `startDownload` received. It also pins the `settings` entry in
// `handleDownloadSelected`'s dependency array, which nothing else asserts.

import { describe, it, expect, vi, afterEach } from "vitest";
import userEvent from "@testing-library/user-event";
import { render, screen, createTestQueryClient } from "@/test/test-utils";
import { queryKeys } from "@/query/queryKeys";
import { EpisodeList } from "./EpisodeList";
import type { AppSettings, DownloadOptions, PlaylistEntry } from "@/types";

const startDownload = vi.hoisted(() => vi.fn().mockResolvedValue("job-1"));
vi.mock("@/api/downloads", () => ({ startDownload }));

const EPISODES: PlaylistEntry[] = [
    {
        index: 1,
        title: "Episode One",
        url: "https://example.com/e1",
        thumbnailUrl: null,
        duration: 60,
        hasSub: false,
        hasDub: false,
    },
];

/// Seed the settings into the cache rather than letting the query resolve
/// mid-test: with `staleTime: Infinity` the value is present at first render,
/// so a click cannot race the fetch and read `undefined` (which would send the
/// `true` default and fail this test for the wrong reason).
function renderWithSettings(embedThumbnail: boolean) {
    const queryClient = createTestQueryClient();
    queryClient.setQueryData(queryKeys.settings(), {
        embed_thumbnail: embedThumbnail,
    } as unknown as AppSettings);

    return render(
        <EpisodeList episodes={EPISODES} playlistUrl="https://example.com/p" playlistTitle="P" />,
        { queryClient },
    );
}

/** The options object the component handed to `startDownload`. */
function sentOptions(): DownloadOptions {
    expect(startDownload).toHaveBeenCalledTimes(1);
    const call = startDownload.mock.calls[0];
    expect(call).toBeDefined();
    return call![1] as DownloadOptions;
}

afterEach(() => {
    startDownload.mockClear();
});

describe("EpisodeList batch download", () => {
    it("sends embedThumbnail=false when the setting is off", async () => {
        const user = userEvent.setup();
        renderWithSettings(false);

        await user.click(screen.getByRole("checkbox", { name: /select all episodes/i }));
        await user.click(screen.getByRole("button", { name: /download selected/i }));

        expect(sentOptions().embedThumbnail).toBe(false);
    });

    it("sends embedThumbnail=true when the setting is on", async () => {
        const user = userEvent.setup();
        renderWithSettings(true);

        await user.click(screen.getByRole("checkbox", { name: /select all episodes/i }));
        await user.click(screen.getByRole("button", { name: /download selected/i }));

        expect(sentOptions().embedThumbnail).toBe(true);
    });

    // The backend resolves these against AppSettings; sending a value here
    // would override the settings layer, which is the #589 defect class.
    it("leaves backend-resolved fields unspecified", async () => {
        const user = userEvent.setup();
        renderWithSettings(true);

        await user.click(screen.getByRole("checkbox", { name: /select all episodes/i }));
        await user.click(screen.getByRole("button", { name: /download selected/i }));

        expect(sentOptions().subtitleLangs).toBeNull();
    });
});
