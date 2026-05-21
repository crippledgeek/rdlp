// E2E support file — loaded before every test.
//
// Mocks window.__TAURI_INTERNALS__ so the Tauri invoke() calls in invokeClient.ts
// resolve with fixture data during Cypress-driven browser tests.

import "./commands";
import "cypress-axe";

// ---------------------------------------------------------------------------
// Tauri IPC mock
// ---------------------------------------------------------------------------

/**
 * Default fixture responses keyed by Tauri command name.
 * Tests can override individual commands by passing overrides to
 * `setupTauriMock()` or `cy.visitWithMock()`.
 */
type InvokeHandler = (args: Record<string, unknown>) => unknown;

const defaultHandlers: Record<string, InvokeHandler> = {
    search_providers: () => [
        { name: "redtube", display_name: "RedTube" },
        { name: "xhamster", display_name: "xHamster" },
        { name: "tnaflix", display_name: "TNAFlix" },
    ],

    search_filters: (_args) => [
        {
            key: "ordering",
            display_name: "Sort",
            allowed_values: [
                { value: "relevance", label: "Relevance" },
                { value: "newest", label: "Newest" },
                { value: "mostviewed", label: "Most Viewed" },
            ],
            default: "relevance",
        },
        {
            key: "period",
            display_name: "Period",
            allowed_values: [
                { value: "alltime", label: "All Time" },
                { value: "weekly", label: "This Week" },
                { value: "monthly", label: "This Month" },
            ],
            default: "alltime",
        },
    ],

    search_content: (args) => {
        const page = (args.page as number) ?? 1;
        if (page === 1) {
            return {
                results: [
                    {
                        video_url: "https://example.com/video/1",
                        title: "Test Video One",
                        thumbnail_url: "https://example.com/thumb1.jpg",
                        duration: 300,
                        view_count: 12345,
                        upload_date: "2024-01-15",
                    },
                    {
                        video_url: "https://example.com/video/2",
                        title: "Test Video Two",
                        thumbnail_url: "https://example.com/thumb2.jpg",
                        duration: 600,
                        view_count: 67890,
                        upload_date: "2024-02-20",
                    },
                ],
                page: 1,
                has_more: true,
                total_estimate: 42,
            };
        }
        return {
            results: [
                {
                    video_url: "https://example.com/video/3",
                    title: "Test Video Three",
                    thumbnail_url: null,
                    duration: 120,
                    view_count: 999,
                    upload_date: "2024-03-10",
                },
            ],
            page: 2,
            has_more: false,
            total_estimate: 42,
        };
    },

    settings: () => ({
        output_dir: "/home/user/Downloads",
        default_remux: null,
        default_extract_audio: null,
        default_subtitle_format: null,
        default_subtitle_langs: [],
        embed_thumbnail: true,
        embed_metadata: true,
        verbose: false,
        default_search_provider: null,
    }),

    update_settings: () => null,

    pick_directory: () => "/home/user/Videos",

    formats: () => ({
        title: "Test Video",
        formats: [
            {
                format_id: "137",
                ext: "mp4",
                format_note: "1080p",
                width: 1920,
                height: 1080,
                fps: 30.0,
                tbr: 4000.0,
                vcodec: "avc1",
                acodec: null,
                filesize: 524288000,
                vbr: 4000.0,
                abr: null,
                asr: null,
                protocol: "https",
                has_video: true,
                has_audio: false,
            },
        ],
        subtitles: [],
        thumbnail_url: "https://example.com/thumb.jpg",
        duration: 600,
    }),

    start_download: () => "job-test-123",

    queue: () => [],

    get_history: () => [],
};

// ---------------------------------------------------------------------------
// Mock setup function — injects __TAURI_INTERNALS__ and
// __TAURI_EVENT_PLUGIN_INTERNALS__ on a window object.
// ---------------------------------------------------------------------------

/**
 * Set up the full Tauri IPC mock on a window.
 * Called from `onBeforeLoad` for every `cy.visit()`.
 */
function setupTauriMock(
    win: Window,
    overrides: Record<string, InvokeHandler> = {},
) {
    const handlers: Record<string, InvokeHandler> = {
        ...defaultHandlers,
        ...overrides,
    };

    let nextEventId = 1;

    // Mock __TAURI_EVENT_PLUGIN_INTERNALS__ so the event module's
    // _unlisten() can call unregisterListener without crashing.
    (win as Window & { __TAURI_EVENT_PLUGIN_INTERNALS__?: unknown })
        .__TAURI_EVENT_PLUGIN_INTERNALS__ = {
            unregisterListener(_event: string, _eventId: number) {
                // no-op in test
            },
        };

    // Mock the __TAURI_INTERNALS__ object that @tauri-apps/api/core reads
    // to dispatch invoke() calls. The real Tauri runtime sets this up;
    // in Cypress we provide a stub that resolves immediately.
    (win as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {
        invoke(command: string, args: Record<string, unknown> = {}): Promise<unknown> {
            // Handle Tauri event plugin commands
            if (command === "plugin:event|listen") {
                return Promise.resolve(nextEventId++);
            }
            if (command === "plugin:event|unlisten") {
                return Promise.resolve();
            }

            const handler = handlers[command];
            if (handler) {
                try {
                    const result = handler(args);
                    return Promise.resolve(result);
                } catch (err) {
                    return Promise.reject(err);
                }
            }
            // Unhandled commands resolve to null to prevent test hangs
            cy.log(`[Tauri mock] Unhandled command: ${command}`);
            return Promise.resolve(null);
        },
        // Additional internals stubs that Tauri plugins may read
        transformCallback(callback: (data: unknown) => void, once?: boolean): number {
            const id = Math.floor(Math.random() * 1000000);
            (win as unknown as Record<string, unknown>)[`_${id}`] = once
                ? (data: unknown) => {
                      delete (win as unknown as Record<string, unknown>)[`_${id}`];
                      callback(data);
                  }
                : callback;
            return id;
        },
        convertFileSrc(src: string): string {
            return src;
        },
        metadata: {
            version: "2.0.0",
            tauriVersion: "2.0.0",
        },
    };
}

// ---------------------------------------------------------------------------
// Inject Tauri mock before each test
// ---------------------------------------------------------------------------

beforeEach(() => {
    cy.visit("/", {
        onBeforeLoad(win) {
            setupTauriMock(win);
        },
    });
});

// ---------------------------------------------------------------------------
// Export for use in tests that need to re-visit with custom handlers
// ---------------------------------------------------------------------------

export { setupTauriMock, defaultHandlers };
export type { InvokeHandler };
