# WIT contract compatibility policy

Governs `rdlp:plugin` version bumps across `types.wit`, `host.wit`,
`extractor.wit`. Applies to every future change to this package, not just
0.5.1.

## 1. Version rule

The Component Model Explainer's "Canonical Interface Name" folds a `0.x.y`
package version to `0.x` — `rdlp:plugin@0.5.0` and `@0.5.1` are the *same*
canonical name and link to each other; `@0.6.0` is a different name and
orphans every 0.5.x plugin. So: **patch = additive** (new functions, new
records, new exports with stubs); **minor = breaking**, accepted to orphan
older plugins; **never ship a pre-release tag** (`0.5.1-rc1` canonicalizes to
its own distinct name, not into `0.5`). Host check
(`loader::check_wit_version_against`): `plugin.major == host.major &&
plugin.minor == host.minor && plugin.patch <= host.patch`.

## 2. What "additive" means here

Never add a field to a shipped record; never change an existing function's
signature. Add a new record or a new function instead. `format`, `info-dict`,
`plugin-info`, `search-query`, `search-page` are frozen as of 0.5.0 — a
patch bump must not touch their fields.

## 3. Exports added after 0.5.0 are optional to the host

`wasmtime-wit-bindgen` 30 requires every world-level export to exist at
`instantiate` time — measured against a 0.5.0-built component bound to the
full `extractor-plugin` world, which fails with exactly:
`no function export `search-filters` found`. `extractor.wit` splits the
contract in two: `extractor-plugin-host` (the frozen 0.5.0 exports) and
`extractor-plugin` (`include`s the host world, adds every post-0.5.0
export: `@since(version = 0.5.1) search-filters`, `@since(version = 0.5.2)
extract-playlist` and `extract-with-metadata`). `lib.rs` binds only the
smaller host world at compile time — this is what lets a 0.5.0 component
instantiate on the current host at all. Each optional export is looked up
by name on the live instance at call time through one shared mechanism,
`adapter::call_export_by_name`, which answers `None` when the component
never declared it; the call site attaches the meaning:
`search_adapter::call_search_filters` answers `[]`,
`playlist_adapter::call_extract_playlist` and
`metadata_adapter::call_extract_with_metadata` hand `None` back so their
caller tries the frozen export instead (§8, §9). The absent export is a
normal outcome, not an error.

## 4. `@since` toolchain acceptance (measured 2026-09-15)

`cargo-component 0.21.1` and `componentize-py 0.17.2` both accepted `@since`
and `include` and built a plugin against this layout with no warning; a
0.5.0-built component instantiated on `extractor-plugin-host` and failed on
`extractor-plugin` with `no function export `search-filters` found` — the
proof the host/full-world split is load-bearing. See the committed fixture
README: `crates/rdlp-plugin/tests/fixtures/example-extractor-0.5.0/README.md`.
Same `@since` pattern wasi-http used across `wasi:http@0.2.1…0.2.8` (spec's
D1 research note).

## 5. What does NOT belong in the host surface

Site-specific obfuscation. `rdlp-crypto` is a **library** a plugin links as
an ordinary crate dependency (no I/O, zero dependencies, wasm-clean),
never a host import — a host import would freeze one site's decryption
scheme into every future ABI version, for every plugin, forever.

The crate ships the small reversible primitives that recur across
video-hosting sites' client-side URL/source obfuscation, each public as "a
number plus its parameters" (mirrors `crates/rdlp-crypto/src/lib.rs`'s own
module list):

- `hash` — `hash::java_string_hash32` (Java-style `String.hashCode` folding
  hash); `hash::fmix32` re-exported from `prng`
- `shuffle` — `shuffle::seeded_shuffle` (seeded Fisher-Yates permutation,
  caller supplies the PRNG draw)
- `transposition` — `transposition::columnar_transpose` /
  `transposition::columnar_untranspose` (columnar transposition cipher)
- `radix` — `radix::to_radix` (arbitrary-base integer encoding, `2..=36`)
  and `radix::Radix` (the validated base); `radix::to_base36` is
  `to_radix(n, Radix::BASE36)`
- `homoglyph` — `homoglyph::HomoglyphTable` and the
  `CYRILLIC_UPPERCASE_TO_LATIN` table (visually-identical character
  substitution)
- `js_int` — `js_int::to_signed_32` (the JS `|0` coercion every
  JS-emulating primitive below needs)
- `prng` — the PRNG algorithm variants: `prng::lcg_step`,
  `prng::weyl_step`, `prng::xorshift` (+ `prng::XorshiftShifts`),
  `prng::rotate_scramble` (+ `prng::Rotation`), `prng::fmix32`,
  `prng::pcg_xsh_rs`, `prng::mxs_mix`, and the composed 7-algorithm
  `ByteGenerator` façade

Site wiring — which algorithm id, which byte offsets, which key
derivation, which homoglyph table — stays in the plugin. The `xhamster`
plugin in rdlp-plugins is the first consumer: it links this crate to
reconstruct its PRNG-based URL decryption, exactly as the in-tree
`megacloud`, `eporner`, and `kvs` decoders do today.

## 6. Deferred to the next minor

Splitting `extract` and `search` into separate WIT interfaces, so a plugin
exports only the capability it provides instead of a stubbed no-op. This
renames exports (a breaking change) and is deferred to the next minor bump.

## 7. Manifest fields added in 0.5.1

0.5.1 adds three `rdlp-plugin-manifest` TOML fields, not WIT records (adding
a field to `plugin-info` itself would be a breaking record change):
`supports_extract` (default `true`), `search_site` (optional), and
`search_claims_override` (default empty; every entry must equal the
plugin's own search site). Canonical manifest bytes include
`supports_extract` only when `false`, `search_site` only when present, and
`search_claims_override` only when non-empty — so a 0.5.0 manifest's
canonical bytes, and its signature over them, stay valid unchanged.

## 8. Playlist (0.5.2)

`extract-playlist: func(url: string, page: u32) -> result<playlist-page,
playlist-error>` (`@since(version = 0.5.2)`, full world only, §3). The
plugin *lists*; the host *resolves*. A `playlist-page` carries `entries`
(`playlist-entry`: `url`, optional `id`/`title`), a 1-indexed `page`,
`has-more`, and the optional playlist-level `playlist-id`,
`playlist-title`, `total-estimate` — read from page one only. Each entry is
resolved by the host through this plugin's own `extract` (yt-dlp
`url_result`, gallery-dl `Message.Queue`); a plugin never resolves items
itself. `playlist-error` is `search-error`'s vocabulary minus `unsupported`
(an absent export already says that) and `cancelled` (the host cancels the
whole call).

**Optional, by name.** `playlist_adapter::call_extract_playlist` resolves
the export on the live instance. Whether the component declares it at all
is read off the component type once at load
(`PluginExtractor::has_extract_playlist`), so a plugin without the export —
and any plugin when `Config::extract_playlist` is `false` — goes straight
to a single `extract` (the `InfoExtractor::extract_playlist` trait default)
with no instantiation spent asking. Otherwise
`PluginExtractor::extract_playlist_via_plugin` fetches page one as a probe:
`unsupported-url` on that page means "not a playlist for this plugin" and
falls back the same way — a per-call cost by nature, since the plugin
decides per URL. A successful page one is handed to the loop, not
re-fetched. Every other domain error on page one propagates (`not-found`
stays `not-found`); only `internal` records a strike
(`playlist_error_to_plugin_error`). A page whose echoed `page` differs from
the one requested is logged on the plugin's target and otherwise converted
unchanged — the host keeps its own count.

**The host loop owns everything after listing**
(`rdlp_extractor::base::common::PagedPlaylist`, `playlist.rs`; the plugin
host is `playlist_adapter::PluginPlaylistSource`). Nothing about range,
concurrency, timeout, or failure policy crosses the WIT boundary:

- *Range.* `Config::{playlist_start, playlist_end, playlist_items}` select
  1-based listing positions; the selection is validated before the probe
  (`validate_selection`), so a malformed `playlist_items` fails before any
  network call. Listing stops at the last needed position, at `!has-more`,
  at an empty page, or at `MAX_PLAYLIST_SIZE` (1000) entries overall. One
  page is additionally capped at `MAX_PLUGIN_PLAYLIST_PAGE_ENTRIES`
  (= `MAX_PLAYLIST_SIZE`) rows, truncated from the tail with one warning.
  Pages after the first are paced by `PAGE_RATE_LIMIT_MS` (500 ms), and
  each page call runs in a fresh store under `LISTING_TIMEOUT` (=
  `SEARCH_TIMEOUT`, 60 s).
- *Concurrency.* `Config::playlist_concurrency`, default
  `DEFAULT_PLAYLIST_CONCURRENCY` = 1 (validated `1..=16`).
- *Per-item timeout.* `Config::playlist_item_timeout` seconds, default
  `DEFAULT_PLAYLIST_ITEM_TIMEOUT_SECS` = 30 (validated `1..=600`). It is
  the budget of the entry's `extract` call itself
  (`PluginExtractor::extract_within`: the runner's tokio timeout AND epoch
  deadline), so an entry has ONE timer; a slow entry is a
  `PluginError::Timeout`. The loop's own guard sits
  `PLAYLIST_ITEM_TIMEOUT_GRACE` (5 s) behind the budget and fires only for
  an implementor that ignores it — never for a plugin.
- *Timeout strikes, once per batch.* A timed-out entry is a strike AT MOST
  ONCE per `extract_playlist` call: the batch's `PluginPlaylistSource`
  holds one gate (`adapter::TimeoutStrikes::OncePer`) that the first
  timed-out entry claims; later timeouts in the same batch still fail
  their entry but are not counted, so a dead upstream under
  `playlist_concurrency = 16` costs the plugin one strike rather than
  disabling it in a single batch. The next batch starts unclaimed, so a
  plugin that keeps timing out across batches still strikes out. Traps and
  `internal` errors are counted every time regardless; a standalone
  `extract`, a search call, and a page fetch run under
  `TimeoutStrikes::Always`.
- *Failure.* `Config::playlist_ignore_errors`, default `true`: a failed or
  timed-out entry is skipped with a warning, and a later page's listing
  failure is warned about and the entries listed so far are resolved.
  `false`: the first failed entry *by listing position* returns its own
  error and nothing is returned; a later page's listing failure returns
  the page's own error before anything is resolved. A first-page failure
  propagates under either policy.
- *Stamping.* Every resolved `InfoDict` gets `playlist_id` from
  `playlist-id`, `playlist_title` from `playlist-title`, `playlist` = the
  title when present else the id (yt-dlp `playlist = playlist_title or
  playlist_id`), `playlist_index` = its 1-based listing position, and
  `playlist_count` = the site's `total-estimate` from page one when
  present (never below the listed count — a site total smaller than what
  was listed is wrong by construction), else the number of entries listed
  (yt-dlp `n_entries`) — never the number selected or resolved.

## 9. Metadata (0.5.2)

`extract-with-metadata: func(url: string) -> result<extraction,
extract-error>` (`@since(version = 0.5.2)`, full world only, §3; the error
is spelled `metadata-extract-error` in `extractor.wit` — a local alias of
`types.extract-error`, the same wire type, because `include` already
imports the bare name). `extract-error` is **not** extended. `extraction`
is the frozen 0.5.0 `info-dict` as `core` plus `info-dict-extra`: the typed
fields `info-dict` cannot carry (`actors`, `channel`, `channel-url`,
`age-limit`, `thumbnails` — `thumbnail` mirrors rdlp's native `Thumbnail`)
and the open tail `extras: list<tuple<string, meta-value>>`. `meta-value`
is typed — `text`, `integer(s64)`, `number(f64)`, `flag`, `text-list` — the
OpenTelemetry stable-attribute subset, never a nested map. A plugin
exporting this implements `extract` as `extract-with-metadata(url).core`.

**Host routing.** `PluginExtractor::extract` tries `extract-with-metadata`
first (`metadata_adapter::call_extract_with_metadata`, by name). Absent →
the frozen `extract`, unchanged. Present → its answer is final: the
extraction is converted (`convert::info_dict_from_extraction`, which
reuses `info_dict_from_wit` for the core and copies the typed fields
across; an empty `thumbnails` becomes `None`), and a domain error maps
through the same `extract_error_to_plugin_error` as `extract` — it is not
a fallback trigger.

**`extras` rules** (`metadata_adapter/extras.rs::extras_from_wit`), applied
in the plugin's order; the first admissible entries are kept:

- *Key.* `^[a-z][a-z0-9-]{0,62}$`, i.e. at most `MAX_METADATA_KEY_BYTES`
  (63) bytes — the DNS-label / Kubernetes-label shape. A duplicate of an
  already-admitted key is refused (first wins).
- *Reserved.* A key that, after folding `-` to `_`, names a typed `InfoDict`
  field is refused with a warning. `InfoDict::extra` is
  `#[serde(flatten)]`, so an admitted key becomes a **top-level key of the
  dict's JSON** (not nested under an `extra` object) and a reserved one
  would overwrite the typed field in every rendering. Reservation is
  decided by a serde probe (`key_is_reserved`), not a hand-kept list, so it
  cannot drift from the struct.
- *Value.* `number` must be finite (JSON has no NaN/∞). A `text` value is
  charged its byte length; a `text-list` is charged the sum of its strings'
  bytes plus `METADATA_LIST_ITEM_BYTES` (8) per element, so its element
  count is bounded too; `integer`/`number`/`flag` are charged
  `SCALAR_VALUE_BYTES` (8) toward the aggregate and are exempt from the
  per-value bound (their size is fixed by the type). Per-value bound:
  `Config::max_metadata_value_bytes`, default
  `DEFAULT_MAX_METADATA_VALUE_BYTES` = 4096 (validated `1..=1_048_576`).
- *Count and aggregate.* At most `Config::max_metadata_extras` entries,
  default `DEFAULT_MAX_METADATA_EXTRAS` = 64 (validated `1..=1024`); key
  bytes plus value bytes of every kept entry at most
  `Config::max_metadata_extras_bytes`, default
  `DEFAULT_MAX_METADATA_EXTRAS_BYTES` = 65 536 (validated
  `1..=16_777_216`). Once the aggregate is crossed no later entry is
  admitted. Both bounds are checked *before* any key or value work on an
  entry, and so is a refusal budget: once more than `max_metadata_extras`
  entries have been refused after per-entry work (malformed, duplicate,
  reserved, or a bad value), the rest of the list is refused as over-count
  with no further work. The reserved-name probe (one `InfoDict`
  deserialisation) runs only after the cheaper checks, once per distinct
  key — so at most `2 × max_metadata_extras + 1` times per extraction
  however many entries a plugin supplies.
- *Lists.* `actors` is capped at `MAX_PLUGIN_ACTORS` (64) and `thumbnails`
  at `MAX_PLUGIN_THUMBNAILS` (64), truncated from the tail with one warning
  each, the same way `extract`'s `formats` is capped.
- *Error text.* Every plugin-authored `detail` in an `extract-error` /
  `playlist-error` / `search-error` case is made sink-safe at the one
  mapping site (`adapter::plugin_detail`): control characters stripped,
  credentials redacted, then cut to `MAX_PLUGIN_ERROR_DETAIL_BYTES` (512).
- *Ids.* `info-dict.id` has every control character replaced by `_` at the
  boundary (`rdlp_redact::text::sanitize_for_line`), and the download
  archive's `archive_key` does the same again: the archive is one
  `{extractor} {id}` record per line, so a line break in an id would write
  a record for another extractor's video.
- *Diagnostics.* One warning per refusal class per extraction on the
  plugin's log target, naming the bound — never the key or value.

The three caps are `MetadataCaps`, built from the call's `Config`
(`MetadataCaps::from(&Config)`; an unset field keeps its default) and set on
the store by `PluginExtractor::extract` before the export runs.

## 10. `display_name` manifest field (0.5.2)

A fourth TOML-only field (`plugin-info` stays frozen, §2/§7):
`display_name: Option<String>`. `Manifest::display_name()` returns it when
set, else `name`. Canonical manifest bytes include it only when set, so
every earlier manifest's bytes and signature stay valid unchanged.
Validation (`validate_display_name`): non-empty, at most
`DISPLAY_NAME_MAX_BYTES` (64) **bytes**, no control characters, no bidi
embedding/override/isolate controls (`U+202A..=U+202E`,
`U+2066..=U+2069` — Trojan Source), no path separator (`/`, `\`); spaces
and mixed case are fine.

It feeds display surfaces: `InfoExtractor::name`
(`PluginExtractor::name`), `InfoDict::extractor` and therefore
`%(extractor)s` (`convert::PluginOrigin::display_name`), and the playlist
loop's log tag (`PagedPlaylist::name`). `%(extractor)s` is the one place a
display surface touches disk — the template renderer makes it ONE output
path component, mapping `/` and `\` to `_` itself — so refusing a
separator at the source is defence in depth, keeping the name an author
wrote the one the user sees; the value is not otherwise a path or a
namespace key.

Identity travels beside it: `convert::info_dict_from_wit` sets
`InfoDict::extractor_key` (yt-dlp `extractor_key`) to the manifest `name`,
and the download-archive token is
`rdlp_api::orchestrator::archive::archive_token_for` = `extractor_key`
when set, else `extractor` (an in-tree extractor's one name). So the
archive, URL and search routing, the trust store, the disabled list, and
the `host-store-kv` namespace are all keyed on `name`; changing
`display_name` never splits an archive. The token is additionally written
ASCII-lowercased and matched case-insensitively (`archive_key`; legacy
cased lines are normalised on read), mirroring yt-dlp's `make_archive_id`
(`f'{ie_key.lower()} {video_id}'`). The id half stays case-sensitive.
