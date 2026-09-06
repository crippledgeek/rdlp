// Tauri IPC mock for Cypress E2E tests.
//
// Mocks window.__TAURI_INTERNALS__ so the Tauri invoke() calls in
// invokeClient.ts resolve with fixture data during browser tests.
//
// Its own module rather than living in support/e2e.ts: commands.ts needs
// `setupTauriMock` for cy.visitWithMock(), and e2e.ts imports commands.ts, so
// keeping it here is what stops that becoming an import cycle.

/** One mocked Tauri command: takes the invoke args, returns the fixture. */
type InvokeHandler = (args: Record<string, unknown>) => unknown;

/**
 * Commands that fell through to the unregistered-command fallback during the
 * current test. Asserted empty in `afterEach` — see the comment there for why
 * rejecting alone is not enough to surface a harness gap.
 */
const unregisteredCommands = new Set<string>();

/**
 * The error both report paths raise — the invoke fallback (which rejects the
 * call) and the `afterEach` gate (which fails the run; see the comment on that
 * hook in ./e2e for the abort scope). One builder rather
 * than two message literals: they state the same fact, so a change to the
 * guidance has to reach both by construction.
 */
function unregisteredCommandError(commands: readonly string[]): Error {
    const subject =
        commands.length === 1 ? "command" : `${commands.length} commands`;
    return new Error(
        `[Tauri mock] Unregistered ${subject} invoked: ${commands.join(", ")}. ` +
            `Add a default handler in cypress/support/tauriMock.ts, or pass one ` +
            `for this test via cy.visitWithMock({ ${commands[0]}: () => ... }).`,
    );
}

/**
 * Default fixture responses keyed by Tauri command name — the handlers every
 * test gets. A single test overrides one by passing it to
 * `cy.visitWithMock({ <command>: () => ... })`.
 */
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

    // Search results render <Thumbnail>, which routes every external HTTPS URL
    // straight through the Rust proxy rather than attempting a direct <img>
    // load — `useProxyFromStart` forces `directFailed` on the first render
    // (Thumbnail.tsx:81), off `shouldProxy` (23-37). Unregistered, this fell through the fallback: the
    // old one fabricated a null that `new Uint8Array(null)` turned into an
    // empty blob, so the thumbnails were broken images the specs never noticed.
    //
    // Real bytes, not an empty buffer — a 1x1 transparent PNG — so the <img>
    // actually decodes. There is no safety net if it does not: the proxy-success
    // branch renders a bare <img> with no onError (Thumbnail.tsx:134), and the
    // bg-muted placeholder needs `proxyFailed` from the QUERY, which a decode
    // failure never sets. An undecodable blob is therefore a permanently broken
    // image that no assertion here can tell apart from a rendered one.
    proxy_thumbnail: () => {
        const PNG_1X1_BASE64 =
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk" +
            "YPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        const binary = atob(PNG_1X1_BASE64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) {
            bytes[i] = binary.charCodeAt(i);
        }
        return bytes.buffer;
    },

    start_download: () => "job-test-123",

    queue: () => [],

    // Registered so the Settings view has codec data. Unregistered, these once
    // fell through to a fallback that resolved `null` — a value the real
    // contract cannot produce: both commands are INFALLIBLE, returning a plain
    // `Vec<_>` (src-tauri/src/commands/codecs.rs:15,27), so not even an error
    // path yields one. `SystemSection` defaulted with a `= []` destructuring,
    // which fires only on undefined, so the fabricated null reached `.length`
    // and crashed the whole Settings view; the a11y spec failed on it and it
    // read as an app bug rather than a harness one (#709). Both halves of that
    // are now fixed — the section uses `?? []` and the fallback rejects — so
    // this registration is what keeps the section MOUNTED, not what keeps it
    // from crashing.
    //
    // These return one codec each rather than `[]` ON PURPOSE. `SystemSection`
    // opens with `if (codecs.length === 0) return null`, so an empty array
    // unmounts the section — the a11y spec would then pass because the codec
    // markup is ABSENT, not because it is accessible, and no regression inside
    // it could ever be caught. The same applies to DownloadConfig's
    // expert-mode codec selectors.
    available_codecs: () => [
        {
            codec: "h264",
            displayName: "H.264 / AVC",
            defaultContainer: "mp4",
            encoders: [
                { encoderName: "libx264", displayName: "x264", speedControls: [] },
            ],
        },
    ],

    available_audio_codecs: () => [
        {
            codec: "aac",
            displayName: "AAC",
            encoders: [{ encoderName: "aac", displayName: "AAC (native)" }],
            supportedContainers: ["mp4", "mkv"],
        },
    ],
};

/**
 * Set up the full Tauri IPC mock on a window.
 * Called from `onBeforeLoad` by `cy.visitWithMock()`, which is the only
 * caller — reach for that rather than this.
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
            // An unregistered command REJECTS, mirroring what the real Tauri
            // runtime does when a command is not on the invoke_handler. The
            // previous fallback resolved `null` for every command alike, and
            // for a VALUE-RETURNING one that is a value the contract cannot
            // produce — it returns a concrete type or an error. (The void
            // commands genuinely do resolve null: `update_settings` above is a
            // faithful fixture for `Result<(), AppError>`. They were never the
            // problem; blanketing them together was.) `invokeTyped<T>` does not
            // validate `T`, so the fabricated null flowed into component code
            // typed as if it could not exist, crashed the whole Settings view
            // once (#709), and read as an application bug for long enough to
            // nearly ship a fourteen-site sweep for it.
            unregisteredCommands.add(command);
            return Promise.reject(unregisteredCommandError([command]));
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

export { setupTauriMock, unregisteredCommands, unregisteredCommandError };
export type { InvokeHandler };
