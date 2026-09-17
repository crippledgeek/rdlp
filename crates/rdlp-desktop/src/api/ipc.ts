// The ONE map of Tauri commands to their argument and result types.
//
// `invokeTyped` (invokeClient.ts) is keyed by this map: a misspelt command, a
// missing or extra argument, a wrongly-typed payload, or a result used under
// the wrong type is a compile error at the call site instead of a runtime
// `undefined` (the SystemSection null-from-stub crash, and downloads.ts once
// declaring `clear_completed_jobs` as `void` where Rust returns `usize`).
//
// Design (docs/development/typed-ipc-command-map-research-2026-09-17.md):
// the key→handler map from typed-rocks `mapped_types.ts` (`[Key in keyof T]`),
// with the argument tuple COMPUTED from the map as in typed-rocks `console.ts`,
// so a `void`-args command takes no second argument. The result type derives
// from the request — the opposite of the `fetch<Todo>(url)` anti-pattern in
// typed-rocks `fetch.ts`, where the caller asserts the response type.
//
// One entry per `commands::…::name` in `src-tauri/src/lib.rs`'s
// `generate_handler![]`; `scripts/check-ipc-command-drift.sh` fails the build
// when the two sets differ. Argument keys are the Rust parameter names in
// camelCase (Tauri's default); result types mirror `-> Result<T, AppError>`
// (`Vec<X>` → `X[]`, `Option<X>` → `X | null`, `()` → `void`, `usize` →
// `number`, `tauri::ipc::Response` → `ArrayBuffer`).

import type {
    AppSettings,
    AudioCodecInfo,
    ContainerFormat,
    DownloadJob,
    DownloadOptions,
    EffectiveNormalize,
    FormatData,
    FormatListResponse,
    LoudnormPreset,
    LoudnormPresetInfo,
    NetworkDefaults,
    PlaylistContext,
    SearchFilter,
    SearchFilterDescriptor,
    SearchPageResponse,
    SearchResultPreview,
    SearchSiteInfo,
    VideoCodecInfo,
} from "../types";

export interface IpcCommands {
    // ---- codecs.rs
    available_codecs: { args: void; result: VideoCodecInfo[] };
    available_audio_codecs: { args: { container: ContainerFormat | null }; result: AudioCodecInfo[] };
    // ---- search.rs
    search_content: {
        args: { query: string; site: string; filters: SearchFilter[]; page: number | undefined };
        result: SearchPageResponse;
    };
    enrich_search_result: { args: { site: string; preview: SearchResultPreview }; result: SearchResultPreview };
    search_providers: { args: void; result: SearchSiteInfo[] };
    search_filters: { args: { site: string }; result: SearchFilterDescriptor[] };
    // ---- download.rs
    start_download: {
        args: { url: string; options: DownloadOptions; title: string | null; playlistContext: PlaylistContext | null };
        /** The job UUID. */
        result: string;
    };
    cancel_download: { args: { jobId: string }; result: void };
    queue: { args: void; result: DownloadJob[] };
    remove_job: { args: { jobId: string }; result: void };
    /** `Result<usize, AppError>`: the number of jobs removed (download.rs). */
    clear_completed_jobs: { args: void; result: number };
    /** The `DownloadOptions` snapshot stored on the job, or `null` if none was recorded (download.rs). */
    job_options: { args: { jobId: string }; result: DownloadOptions | null };
    // ---- formats/mod.rs
    formats: { args: { url: string }; result: FormatListResponse };
    validate_format_expression: { args: { expression: string; formats: FormatData[] }; result: string[] };
    // ---- settings.rs
    settings: { args: void; result: AppSettings };
    network_defaults: { args: void; result: NetworkDefaults };
    effective_normalize: { args: { preset: LoudnormPreset | null }; result: EffectiveNormalize };
    loudnorm_presets: { args: void; result: LoudnormPresetInfo[] };
    update_settings: { args: { settings: AppSettings }; result: void };
    pick_directory: { args: void; result: string | null };
    reveal_in_folder: { args: { path: string }; result: void };
    // ---- thumbnail.rs
    /** `tauri::ipc::Response` carries the image bytes raw, bypassing JSON. */
    proxy_thumbnail: { args: { url: string }; result: ArrayBuffer };
}

export type IpcCommand = keyof IpcCommands;
export type IpcArgs<K extends IpcCommand> = IpcCommands[K]["args"];
export type IpcResult<K extends IpcCommand> = Promise<IpcCommands[K]["result"]>;

/**
 * The rest-parameter tuple for a command: empty for a `void`-args command, one
 * named `args` element otherwise — so `invokeTyped("settings")` compiles and
 * `invokeTyped("effective_normalize")` is `TS2554 Expected 2 arguments`.
 */
export type IpcArgTuple<K extends IpcCommand> = IpcArgs<K> extends void ? [] : [args: IpcArgs<K>];

/**
 * A per-command stub table for tests: each key's stub returns THAT command's
 * result type, so a cross-wired stub (`settings` returning a
 * `NetworkDefaults`) is a compile error, not a runtime crash. Write the table
 * as `const stubs = { … } satisfies IpcStubTable` and look it up in a typed
 * `mockImplementation`.
 */
export type IpcStubTable = { readonly [K in IpcCommand]?: () => IpcResult<K> };
