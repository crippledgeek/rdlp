# Network Timeout CLI Flags + Desktop Settings — Design

**Issue:** [#278](https://github.com/crippledgeek/rdlp/issues/278)
**Date:** 2026-05-04
**Status:** Draft (pending spec review)

## Summary

`rdlp_types::Config` exposes three HTTP-client timeout knobs (`socket_timeout`, `read_timeout`, `pool_idle_timeout`) that are currently TOML-only. This spec adds:

1. CLI flags for all three on `rdlp-cli`.
2. Equivalent fields in `rdlp-desktop`'s Settings → Network panel.
3. Zod-based client-side input parsing on the frontend so invalid keystrokes never reach the IPC boundary.

Validation of the resulting `Config` continues to flow through the single source of truth, `Config::validate()`. The frontend's zod schema is **input parsing**, not duplicate validation — its job is to coerce strings to integers and reject UI-level garbage (negative numbers, non-numerics) for immediate user feedback.

The issue's premise that we are "mirroring the existing `--socket-timeout`" is false: that flag does not exist on the CLI today either. Per session resolution we therefore add all three flags as one coherent group, not just the two named in the issue.

## Non-goals

- No change to `rdlp-http` / `wreq` plumbing — `Config` already feeds the client builder.
- No millisecond precision; seconds only.
- No CLI/Settings exposure of any other `Config` field that is currently TOML-only.
- No "Advanced" accordion / disclosure in Settings (rationale below).

## Background

### Current state

`crates/rdlp-types/src/config.rs`:

| Field | Type | Default | Sentinel |
|-------|------|---------|----------|
| `socket_timeout` | `Option<u64>` | `Some(30)` | none |
| `read_timeout` | `Option<u64>` | `None` (= use http default) | none |
| `pool_idle_timeout` | `Option<u64>` | `None` (= use reqwest default) | `Some(0)` = disable eviction (keep idle sockets forever, mapped to `pool_idle_timeout(None)` at the wreq/reqwest layer) |

All three are validated by `Config::validate()` returning `ConfigValidationError`.

`crates/rdlp-cli/src/args.rs` exposes none of them. `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx` exposes Proxy + Rate Limit only; `AppSettings` mirrors a subset of `Config`.

### UX research (canonical pattern)

The researcher report consulted VLC, mpv, yt-dlp, and Firefox. Findings:

- **Units:** seconds, integer, plain `<input type="number">`. No comparator uses ms/s toggle, sliders, or preset dropdowns for these knobs.
- **Labels:** Firefox uses "connection-timeout" / "keep-alive.timeout"; yt-dlp uses `--socket-timeout SECONDS`; full English nouns are clearer than abbreviations.
- **`0 = disable` sentinel:** no surveyed app uses bare `0` as a sentinel in a numeric input. The canonical pattern is checkbox + numeric input where the checkbox toggles the duration's applicability.
- **Placement:** all comparators bury timeouts behind some "Advanced" disclosure (VLC "Show All", Firefox about:config, mpv config-file only).

### Why no Advanced disclosure here

rdlp-desktop's Settings view is already the advanced surface — there is no Simple/All split, every section sits on one scrollable page, and the existing Network section only has 2 fields. Adding a second nested disclosure inside an already-buried panel adds friction without payoff. If the panel grows past ~6 fields later, factor an Advanced subsection then.

## CLI design

### Flags

All three live in `crates/rdlp-cli/src/args.rs`, in the same network-options group as `--proxy`:

| Flag | Type | Maps to | Notes |
|------|------|---------|-------|
| `--socket-timeout SECS` | `Option<u64>` | `Config.socket_timeout` | New |
| `--read-timeout SECS` | `Option<u64>` | `Config.read_timeout` | New |
| `--pool-idle-timeout SECS` | `Option<u64>` | `Config.pool_idle_timeout` | New; `0` keeps eviction-disable sentinel |

`SECS` is parsed as `u64` (clap default). Negative values rejected by clap before reaching us. `0` is accepted on all three; for `--socket-timeout` and `--read-timeout` the value passes through to the validation layer (where `Config::validate()` may reject it — see Validation below); for `--pool-idle-timeout` `0` is the documented sentinel.

### Merge semantics

`crates/rdlp-cli/src/config.rs` merges CLI args into the loaded `Config`. CLI-supplied values override TOML; absent flags (`None`) preserve TOML / default. This matches the existing pattern for `--proxy`.

### Validation

`Config::validate()` runs once at the end of merge. Existing variants (`SocketTimeoutZero`, `ReadTimeoutZero` — confirm exact names during impl) continue to be the single source of truth. No new validation logic on the CLI side.

## Desktop design

### `AppSettings` extension

`crates/rdlp-desktop/src-tauri/src/state/app_settings.rs` gains three new `Option<u64>` fields, serialized in camelCase:

```rust
pub socket_timeout: Option<u64>,    // → socketTimeout
pub read_timeout: Option<u64>,      // → readTimeout
pub pool_idle_timeout: Option<u64>, // → poolIdleTimeout
```

Defaults: `None` for all three (i.e. "use whatever the Config default is at the time of merge"). The existing `Config`-merge path picks them up.

### Frontend types

`crates/rdlp-desktop/src/types/index.ts` `AppSettings` adds:

```ts
socket_timeout: number | null;
read_timeout: number | null;
pool_idle_timeout: number | null;
```

(snake_case retained because `AppSettings` doesn't carry `#[serde(rename_all = "camelCase")]` today — confirm during impl and choose one consistent style.)

### UI layout

Extend `NetworkSection.tsx`'s existing 2-col grid with a new row pair for the timeouts, plus a third full-width row for the idle-connection control:

```
┌────────────────────────────────┬────────────────────────────────┐
│ Proxy                          │ Rate Limit                     │
├────────────────────────────────┼────────────────────────────────┤
│ Connection Timeout             │ Read Timeout                   │
│ [ 30 ] s                       │ [ 60 ] s                       │
│  helper: connect / handshake   │  helper: per-read idle         │
├────────────────────────────────┴────────────────────────────────┤
│ ☑ Evict idle connections after [ 90 ] s                         │
│   helper: when off, idle keep-alive sockets are kept until OS   │
│   closes them                                                   │
└─────────────────────────────────────────────────────────────────┘
```

### Components

- **Connection / Read Timeout** — shadcn `Input type="number" min="0" inputMode="numeric"` with the existing `font-mono text-xs` class. A sibling `<span className="settings-suffix">s</span>` renders the unit. `placeholder` shows the field's effective default ("30", "60").
- **Idle Connection Timeout** — Jolly UI `Checkbox` (preferred over shadcn for consistency with project rule on interactive components) bound to a `evictIdle: boolean` derived state, plus a numeric `Input` whose `disabled` mirrors `!evictIdle`. The control wraps both in a single `<div>` with a shared `aria-describedby` helper-text id.

### Helper text

Each input gets `<p className="settings-helper text-xs ..." id="<field>-help">` linked via `aria-describedby`. Color uses `--text-muted` (5.3:1 AA, per project CSS token rules). Helper copy:

- Connection Timeout — "Time to establish a connection to the server."
- Read Timeout — "Maximum gap between bytes during a download."
- Evict idle connections — "When off, idle keep-alive connections stay open until the OS closes them."

### Mapping rules (form → AppSettings)

`onSubmit` (or `onChange`, matching the existing draft pattern in `SettingsView`):

| Control state | `socket_timeout` / `read_timeout` |
|---|---|
| Empty input | `null` |
| Numeric `n ≥ 1` | `n` |
| Numeric `0` | rejected by zod; submit blocked |

| Control state | `pool_idle_timeout` |
|---|---|
| Checkbox **on**, input empty | `null` (use default) |
| Checkbox **on**, input `n ≥ 1` | `n` |
| Checkbox **off** | `0` (the sentinel; user never sees the literal `0`) |
| Checkbox **on**, input `0` | rejected by zod with hint "Use the checkbox to keep connections forever" |

### Zod schema (frontend input parsing only)

`crates/rdlp-desktop/src/views/settings/networkSchema.ts`:

```ts
import { z } from "zod";

const positiveSeconds = z.preprocess(
    (v) => (v === "" || v === null ? null : Number(v)),
    z.union([z.null(), z.number().int().positive().max(86_400)]),
);

export const networkTimeoutsSchema = z.object({
    socket_timeout: positiveSeconds,
    read_timeout: positiveSeconds,
    pool_idle_timeout: positiveSeconds, // 0 sentinel handled at the form-mapping layer
    evict_idle: z.boolean(),
});
```

Notes:

- `max(86_400)` is a UI-level sanity ceiling (24h) so a typo doesn't silently submit `9999999`. `Config::validate()` may apply a different ceiling or none — frontend's job is to fail fast on obviously wrong input, not to mirror the backend's rule set.
- Zod's role: **type coerce + reject UI-level garbage**. It does not duplicate `Config::validate()`'s integrity checks; failing this gate is a UX shortcut so the user sees "must be a positive integer" inline instead of a Tauri command error.
- The `evict_idle` boolean is a UI-only field; the IPC layer never sees it. The form-mapping layer (above) collapses `evict_idle === false` to `pool_idle_timeout = 0`.

### Accessibility

- `<Label htmlFor="…">` pairs with each input `id` (matches existing `htmlFor="proxy"` pattern at `NetworkSection.tsx:27`).
- Helper text id linked via `aria-describedby`.
- Checkbox + numeric input share an `aria-controls` relationship so AT announces them as a group.
- `disabled` attribute (not just visual grayout) on the idle-timeout numeric input when checkbox is off, so Tab order skips it.
- Cypress a11y suite (`cypress/e2e/a11y.cy.ts`) MUST stay green; the allowlist (`cypress/support/a11y-allowlist.ts`) MUST stay empty.

## Testing

Per `bug-fix-requires-failing-test.md`: at least one positive test plus exhaustive negative coverage proportional to the change surface.

### CLI tests (`crates/rdlp-cli/src/config_tests.rs`)

Positive:
- `--socket-timeout 5 --read-timeout 10 --pool-idle-timeout 90` → all three fields populated on the merged `Config`.

Negative (each must fail against unpatched code, then pass after):
- Negative value (`--socket-timeout=-1`) — clap rejects before merge.
- Non-numeric (`--socket-timeout=abc`) — clap rejects.
- `--pool-idle-timeout 0` accepted (sentinel preserved).
- `--socket-timeout 0` rejected by `Config::validate()` (existing rule — confirm during impl).
- Unspecified flag → field stays `None`, TOML value preserved.
- CLI overrides TOML when both supply a value.

### Desktop frontend tests (vitest)

`NetworkSection.test.tsx`:
- Renders 3 new controls with correct labels and `htmlFor` association.
- Numeric input accepts integers; `onChange` updates draft.
- Checkbox toggle disables the idle-timeout numeric input.
- Form-mapping: checkbox off + any input → `pool_idle_timeout = 0` on submitted draft.
- Form-mapping: checkbox on + empty input → `pool_idle_timeout = null`.
- Zod rejects negative numbers; inline error visible.
- Zod rejects `pool_idle_timeout = 0` with "use the checkbox" hint.
- `aria-describedby` links input → helper text.

### Desktop backend tests (`app_settings.rs`)

- Round-trip: `AppSettings` with timeout fields serialized → deserialized preserves values.
- `apply_settings_to_config` (or merge equivalent) propagates all three fields.

### E2E

Cypress a11y suite remains green over Settings view.

## Files touched

| File | Change |
|---|---|
| `crates/rdlp-cli/src/args.rs` | Add 3 clap flags. |
| `crates/rdlp-cli/src/config.rs` | Wire flags → `Config`. |
| `crates/rdlp-cli/src/config_tests.rs` | New tests. |
| `crates/rdlp-desktop/src-tauri/src/state/app_settings.rs` | Add 3 fields + defaults + tests. |
| `crates/rdlp-desktop/src-tauri/src/commands/settings.rs` | (No change unless merge path needs touching.) |
| `crates/rdlp-desktop/src/types/index.ts` | Add 3 fields to `AppSettings`. |
| `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx` | Add 3 controls + zod-driven validation. |
| `crates/rdlp-desktop/src/views/settings/networkSchema.ts` | New: zod schema + form-mapping helpers. |
| `crates/rdlp-desktop/src/views/settings/NetworkSection.test.tsx` | New: vitest. |

## Open questions

None — all dimensions resolved during brainstorming. Implementation plan to be authored next via `superpowers:writing-plans`.

## References

- Issue [#278](https://github.com/crippledgeek/rdlp/issues/278)
- PR #269 (introduced the underlying `Config` fields)
- yt-dlp `--socket-timeout` ([docs](https://www.mintlify.com/yt-dlp/yt-dlp/cli/network-options))
- mpv `--network-timeout` (default 60s, range `0..`)
- Firefox `network.http.connection-timeout` (90s default), `network.http.keep-alive.timeout` (300s default)
- VLC `ipv4-timeout` (5000ms default; ms-only)
- NN/g — [Toggle Switch Guidelines](https://www.nngroup.com/articles/toggle-switch-guidelines/)
