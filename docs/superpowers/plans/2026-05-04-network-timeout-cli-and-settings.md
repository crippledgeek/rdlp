# Network Timeout CLI Flags + Desktop Settings — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose `socket_timeout`, `read_timeout`, and `pool_idle_timeout` on the `rdlp` CLI as flags and in the `rdlp-desktop` Settings → Network panel, so users can tune HTTP timeouts without hand-editing TOML.

**Architecture:** Three thin passthrough layers — CLI clap args → `Config` (rdlp-cli/config.rs); rdlp-api `NetworkOptions` → `Config` (already used by desktop); `AppSettings` → `NetworkOptions` (desktop merge path). Frontend uses zod for input parsing only; `Config::validate()` remains the single source of truth for value-range validation.

**Tech Stack:** Rust 1.85 edition 2024, clap 4.5, Tauri v2, React 19, TypeScript, zod 3.25, Jolly UI (React Aria) + shadcn primitives.

**Spec:** `docs/superpowers/specs/2026-05-04-network-timeout-cli-and-settings-design.md`

**Branch:** `feature/network-timeout-cli-and-settings` (already created from `develop`).

---

## Backend value ranges (load-bearing)

`Config::validate()` at `crates/rdlp-types/src/config.rs:524-548` enforces:

| Field | Allowed range | Notes |
|---|---|---|
| `socket_timeout` | `Some(1..=300)` | `0` is **rejected**. |
| `read_timeout` | `Some(1..=600)` | `0` is **rejected**. |
| `pool_idle_timeout` | `Some(0..=3600)` | `0` is the sentinel meaning "disable eviction". Translated to `pool_idle_timeout(None)` inside `HttpClientConfig::from_rdlp_config` (`crates/rdlp-http/src/config.rs:87-89`). |

Frontend zod uses these same ranges so users see inline errors instead of "invalid_request" Tauri errors.

---

## Files touched

| File | Reason |
|---|---|
| `crates/rdlp-api/src/request.rs` | Add `read_timeout_secs` and `pool_idle_timeout_secs` fields to `NetworkOptions`. Fix existing doc-comment on `timeout_secs` (currently mislabels it). |
| `crates/rdlp-api/src/merge/mod.rs` | Merge the two new fields into `Config`. |
| `crates/rdlp-api/src/merge/tests_postprocess_network.rs` | Tests for the new merges. |
| `crates/rdlp-cli/src/args.rs` | Add `--socket-timeout`, `--read-timeout`, `--pool-idle-timeout` flags. |
| `crates/rdlp-cli/src/config.rs` | Wire the three flags into `Config`. |
| `crates/rdlp-cli/src/config_tests.rs` | CLI parsing/merge tests. |
| `crates/rdlp-desktop/src-tauri/src/state/app_settings.rs` | Add three `Option<u64>` fields with `#[serde(default)]`. |
| `crates/rdlp-desktop/src-tauri/src/commands/download.rs` | Merge `AppSettings` timeout fields into `NetworkOptions`. |
| `crates/rdlp-desktop/src/types/index.ts` | Mirror the three new fields in the TS `AppSettings`. |
| `crates/rdlp-desktop/src/views/settings/networkSchema.ts` | **New** — zod schema for network timeout inputs. |
| `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx` | Add three controls (two raw numeric, one checkbox-gated numeric). |
| `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.test.tsx` | **New** — vitest covering controls + zod + form mapping. |

---

## Task 1 — Extend `NetworkOptions` with read/pool-idle timeout fields

**Files:**
- Modify: `crates/rdlp-api/src/request.rs:232-248`
- Modify: `crates/rdlp-api/src/merge/mod.rs:148-175`
- Test: `crates/rdlp-api/src/merge/tests_postprocess_network.rs`

- [ ] **Step 1: Write the failing tests**

Append to `crates/rdlp-api/src/merge/tests_postprocess_network.rs`:

```rust
#[test]
fn network_options_merges_read_timeout() {
    use rdlp_types::Config;
    let mut config = Config::default();
    let opts = NetworkOptions {
        read_timeout_secs: Some(45),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.read_timeout, Some(45));
}

#[test]
fn network_options_merges_pool_idle_timeout_positive() {
    use rdlp_types::Config;
    let mut config = Config::default();
    let opts = NetworkOptions {
        pool_idle_timeout_secs: Some(120),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.pool_idle_timeout, Some(120));
}

#[test]
fn network_options_merges_pool_idle_timeout_zero_sentinel() {
    use rdlp_types::Config;
    let mut config = Config::default();
    let opts = NetworkOptions {
        pool_idle_timeout_secs: Some(0),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.pool_idle_timeout, Some(0));
}

#[test]
fn network_options_none_preserves_base_timeouts() {
    use rdlp_types::Config;
    let mut config = Config {
        read_timeout: Some(99),
        pool_idle_timeout: Some(99),
        ..Config::default()
    };
    NetworkOptions::default().merge_into(&mut config);
    assert_eq!(config.read_timeout, Some(99));
    assert_eq!(config.pool_idle_timeout, Some(99));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p rdlp-api network_options_merges_read_timeout network_options_merges_pool_idle_timeout_positive network_options_merges_pool_idle_timeout_zero_sentinel network_options_none_preserves_base_timeouts`
Expected: FAIL — `read_timeout_secs` / `pool_idle_timeout_secs` fields don't exist on `NetworkOptions`.

- [ ] **Step 3: Add the fields to `NetworkOptions`**

In `crates/rdlp-api/src/request.rs`, replace the existing `NetworkOptions` struct (currently lines 232-248) with:

```rust
#[derive(Debug, Clone, Default)]
pub struct NetworkOptions {
    /// Maximum number of retries for failed requests. `None` preserves base config.
    pub retries: Option<u32>,
    /// Connect/handshake timeout in seconds. Maps to `Config::socket_timeout`.
    /// `None` preserves base config. Allowed range: 1..=300.
    pub timeout_secs: Option<u64>,
    /// Per-read idle timeout in seconds. Maps to `Config::read_timeout`.
    /// `None` preserves base config. Allowed range: 1..=600.
    pub read_timeout_secs: Option<u64>,
    /// Idle keep-alive socket eviction timeout in seconds. Maps to
    /// `Config::pool_idle_timeout`. `None` preserves base config.
    /// `Some(0)` is the documented sentinel meaning "disable eviction
    /// (keep idle sockets forever)"; allowed range: 0..=3600.
    pub pool_idle_timeout_secs: Option<u64>,
    /// Number of concurrent download fragments/chunks. `None` preserves base config.
    pub concurrent_fragments: Option<u32>,
    /// Download rate limit in bytes per second.
    pub rate_limit: Option<u64>,
    /// HTTP/SOCKS proxy URL (e.g. `"http://proxy:3128"`, `"socks5://proxy:1080"`). `None` preserves base config.
    pub proxy: Option<String>,
    /// Browser to extract cookies from.
    pub cookies_from_browser: Option<BrowserType>,
    /// Path to a Netscape-format cookies file.
    pub cookies_file: Option<PathBuf>,
}
```

Note: `timeout_secs`'s doc-comment is fixed in this edit ("Per-read idle timeout" → "Connect/handshake timeout") because the previous wording mislabeled the mapping.

- [ ] **Step 4: Wire the new fields into `MergeOverrides`**

In `crates/rdlp-api/src/merge/mod.rs`, replace the `MergeOverrides for NetworkOptions` block (currently lines 148+) so it becomes:

```rust
impl MergeOverrides for NetworkOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(v) = self.retries {
            config.retries = v as usize;
        }
        if let Some(v) = self.timeout_secs {
            config.socket_timeout = Some(v);
        }
        if let Some(v) = self.read_timeout_secs {
            config.read_timeout = Some(v);
        }
        if let Some(v) = self.pool_idle_timeout_secs {
            config.pool_idle_timeout = Some(v);
        }
        if let Some(v) = self.concurrent_fragments {
            config.concurrent_fragments = v as usize;
        }
        if let Some(v) = self.rate_limit {
            config.rate_limit = Some(v);
        }
        if let Some(ref v) = self.proxy {
            config.proxy = Some(v.clone());
        }
        if let Some(v) = self.cookies_from_browser {
            config.cookies_from_browser = Some(v);
        }
        if let Some(ref v) = self.cookies_file {
            config.cookies_file = Some(v.clone());
        }
    }
}
```

(Preserve the existing tail of the function — fields below `cookies_file` if any. Verify in the file before saving.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p rdlp-api network_options_merges_`
Expected: 4 PASS.

Run the full crate too: `cargo test -p rdlp-api`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/rdlp-api/src/request.rs crates/rdlp-api/src/merge/mod.rs crates/rdlp-api/src/merge/tests_postprocess_network.rs
git commit -m "feat(api): add read_timeout_secs and pool_idle_timeout_secs to NetworkOptions"
```

---

## Task 2 — Add three CLI flags

**Files:**
- Modify: `crates/rdlp-cli/src/args.rs:276-291`
- Modify: `crates/rdlp-cli/src/config.rs:272-294`
- Test: `crates/rdlp-cli/src/config_tests.rs`

- [ ] **Step 1: Write failing CLI parse tests**

Append to `crates/rdlp-cli/src/config_tests.rs`. (If the file uses `clap::Parser::try_parse_from(...)` for existing tests, mirror that style. Look near other proxy-related tests for the idiom.)

```rust
#[test]
fn cli_socket_timeout_flag_sets_config_field() {
    use crate::args::Cli;
    use clap::Parser;
    let cli = Cli::try_parse_from(["rdlp", "--socket-timeout", "45", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(cli.socket_timeout, Some(45));
}

#[test]
fn cli_read_timeout_flag_sets_config_field() {
    use crate::args::Cli;
    use clap::Parser;
    let cli = Cli::try_parse_from(["rdlp", "--read-timeout", "120", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(cli.read_timeout, Some(120));
}

#[test]
fn cli_pool_idle_timeout_flag_accepts_zero_sentinel() {
    use crate::args::Cli;
    use clap::Parser;
    let cli = Cli::try_parse_from(["rdlp", "--pool-idle-timeout", "0", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(cli.pool_idle_timeout, Some(0));
}

#[test]
fn cli_pool_idle_timeout_flag_accepts_positive() {
    use crate::args::Cli;
    use clap::Parser;
    let cli = Cli::try_parse_from(["rdlp", "--pool-idle-timeout", "300", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(cli.pool_idle_timeout, Some(300));
}

#[test]
fn cli_timeout_flags_unset_default_to_none() {
    use crate::args::Cli;
    use clap::Parser;
    let cli = Cli::try_parse_from(["rdlp", "https://example.com/x"]).expect("parse should succeed");
    assert!(cli.socket_timeout.is_none());
    assert!(cli.read_timeout.is_none());
    assert!(cli.pool_idle_timeout.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p rdlp-cli cli_socket_timeout_flag cli_read_timeout_flag cli_pool_idle_timeout_flag cli_timeout_flags_unset`
Expected: FAIL — `socket_timeout` / `read_timeout` / `pool_idle_timeout` are not fields on `Cli`.

- [ ] **Step 3: Add the flags to `Cli`**

In `crates/rdlp-cli/src/args.rs`, locate the `// === Network options ===` block (around line 276) and insert the three flags immediately after the existing `proxy` field:

```rust
    /// Connect/handshake timeout in seconds (range: 1..=300).
    #[arg(long, value_name = "SECS")]
    pub socket_timeout: Option<u64>,

    /// Per-read idle timeout in seconds (range: 1..=600).
    #[arg(long, value_name = "SECS")]
    pub read_timeout: Option<u64>,

    /// Idle keep-alive socket eviction timeout in seconds. Use `0` to keep
    /// idle connections forever (range: 0..=3600).
    #[arg(long, value_name = "SECS")]
    pub pool_idle_timeout: Option<u64>,
```

- [ ] **Step 4: Run the parse tests to verify they pass**

Run: `cargo test -p rdlp-cli cli_socket_timeout_flag cli_read_timeout_flag cli_pool_idle_timeout_flag cli_timeout_flags_unset`
Expected: 5 PASS.

- [ ] **Step 5: Write a failing merge test (CLI → Config)**

Append to `crates/rdlp-cli/src/config_tests.rs`. (Mirror the existing pattern used to test `--proxy` merging — find a function like `test_proxy_merge_into_config` for the helper signature.)

```rust
#[test]
fn cli_timeout_flags_merge_into_config() {
    use crate::args::Cli;
    use crate::config::merge_args_into_config;
    use clap::Parser;
    use rdlp_types::Config;

    let cli = Cli::try_parse_from([
        "rdlp",
        "--socket-timeout", "45",
        "--read-timeout", "200",
        "--pool-idle-timeout", "0",
        "https://example.com/x",
    ])
    .expect("parse should succeed");

    let mut config = Config::default();
    merge_args_into_config(&cli, &mut config, Default::default()).expect("merge should succeed");

    assert_eq!(config.socket_timeout, Some(45));
    assert_eq!(config.read_timeout, Some(200));
    assert_eq!(config.pool_idle_timeout, Some(0));
}

#[test]
fn cli_timeout_flags_unset_preserve_config() {
    use crate::args::Cli;
    use crate::config::merge_args_into_config;
    use clap::Parser;
    use rdlp_types::Config;

    let cli = Cli::try_parse_from(["rdlp", "https://example.com/x"]).expect("parse");
    let mut config = Config {
        read_timeout: Some(77),
        pool_idle_timeout: Some(88),
        ..Config::default()
    };
    merge_args_into_config(&cli, &mut config, Default::default()).expect("merge");
    assert_eq!(config.read_timeout, Some(77));
    assert_eq!(config.pool_idle_timeout, Some(88));
}
```

If `merge_args_into_config`'s exact signature differs, adjust the call site to match. The signature in `crates/rdlp-cli/src/config.rs` is the source of truth — read it before writing this test.

- [ ] **Step 6: Run merge tests to verify they fail**

Run: `cargo test -p rdlp-cli cli_timeout_flags_merge_into_config cli_timeout_flags_unset_preserve_config`
Expected: FAIL — `merge_args_into_config` doesn't read the new flags yet.

- [ ] **Step 7: Wire the flags into `merge_args_into_config`**

In `crates/rdlp-cli/src/config.rs`, locate the proxy-merge block (around line 272). Insert immediately after `config.proxy = Some(proxy.clone());`:

```rust
    if let Some(secs) = args.socket_timeout {
        config.socket_timeout = Some(secs);
    }
    if let Some(secs) = args.read_timeout {
        config.read_timeout = Some(secs);
    }
    if let Some(secs) = args.pool_idle_timeout {
        config.pool_idle_timeout = Some(secs);
    }
```

Do **not** add value-range validation here — `Config::validate()` is the single source of truth and runs at the end of `merge_args_into_config`.

- [ ] **Step 8: Run the merge tests to verify they pass**

Run: `cargo test -p rdlp-cli cli_timeout_flags_merge_into_config cli_timeout_flags_unset_preserve_config`
Expected: 2 PASS.

Run full crate too: `cargo test -p rdlp-cli`
Expected: all green.

- [ ] **Step 9: Negative test — out-of-range value rejected by validate**

Append to `crates/rdlp-cli/src/config_tests.rs`:

```rust
#[test]
fn cli_socket_timeout_zero_is_rejected_by_validate() {
    use crate::args::Cli;
    use crate::config::merge_args_into_config;
    use clap::Parser;
    use rdlp_types::Config;

    let cli = Cli::try_parse_from(["rdlp", "--socket-timeout", "0", "https://example.com/x"])
        .expect("parse");
    let mut config = Config::default();
    let res = merge_args_into_config(&cli, &mut config, Default::default());
    // Either merge_args_into_config calls validate internally and fails, OR validate is
    // called after merge by the caller. Test the post-merge validate state explicitly.
    if res.is_ok() {
        assert!(config.validate().is_err(), "Config::validate must reject socket_timeout=0");
    }
}

#[test]
fn cli_pool_idle_timeout_above_max_is_rejected_by_validate() {
    use crate::args::Cli;
    use crate::config::merge_args_into_config;
    use clap::Parser;
    use rdlp_types::Config;

    let cli = Cli::try_parse_from(["rdlp", "--pool-idle-timeout", "9999", "https://example.com/x"])
        .expect("parse");
    let mut config = Config::default();
    let _ = merge_args_into_config(&cli, &mut config, Default::default());
    assert!(config.validate().is_err(), "Config::validate must reject pool_idle_timeout > 3600");
}
```

- [ ] **Step 10: Run negative tests**

Run: `cargo test -p rdlp-cli cli_socket_timeout_zero_is_rejected cli_pool_idle_timeout_above_max`
Expected: 2 PASS.

- [ ] **Step 11: Commit**

```bash
cargo fmt
git add crates/rdlp-cli/src/args.rs crates/rdlp-cli/src/config.rs crates/rdlp-cli/src/config_tests.rs
git commit -m "feat(cli): add --socket-timeout, --read-timeout, --pool-idle-timeout flags (#278)"
```

---

## Task 3 — Add three fields to `AppSettings` (desktop backend)

**Files:**
- Modify: `crates/rdlp-desktop/src-tauri/src/state/app_settings.rs:23-100, 180-216, 320-358`

- [ ] **Step 1: Write the failing tests**

Append inside the `mod tests { ... }` block in `crates/rdlp-desktop/src-tauri/src/state/app_settings.rs`:

```rust
#[test]
fn test_default_timeout_fields_are_none() {
    let s = AppSettings::default();
    assert!(s.socket_timeout.is_none());
    assert!(s.read_timeout.is_none());
    assert!(s.pool_idle_timeout.is_none());
}

#[test]
fn test_timeout_fields_round_trip_json() {
    let s = AppSettings {
        socket_timeout: Some(45),
        read_timeout: Some(120),
        pool_idle_timeout: Some(0),
        ..AppSettings::default()
    };
    let json = serde_json::to_string(&s).expect("serialize");
    let back: AppSettings = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.socket_timeout, Some(45));
    assert_eq!(back.read_timeout, Some(120));
    assert_eq!(back.pool_idle_timeout, Some(0));
}

#[test]
fn test_legacy_settings_json_without_timeout_fields_loads() {
    // Older settings.json files won't have these keys; serde(default) must populate them as None.
    let json = r#"{"output_dir":".","embed_thumbnail":true,"embed_metadata":false,"verbose":false,"default_subtitle_langs":[]}"#;
    let s: AppSettings = serde_json::from_str(json).expect("must load legacy json");
    assert!(s.socket_timeout.is_none());
    assert!(s.read_timeout.is_none());
    assert!(s.pool_idle_timeout.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p rdlp-desktop test_default_timeout_fields_are_none test_timeout_fields_round_trip_json test_legacy_settings_json_without_timeout_fields_loads`
Expected: FAIL — fields missing from struct.

- [ ] **Step 3: Add the three fields**

In `crates/rdlp-desktop/src-tauri/src/state/app_settings.rs`, inside the `pub struct AppSettings { ... }` block (currently ends around line 100), add immediately before the closing brace:

```rust
    /// Connect/handshake timeout in seconds. `None` uses default (30).
    /// Range enforced by `Config::validate()`: 1..=300.
    #[serde(default)]
    pub socket_timeout: Option<u64>,
    /// Per-read idle timeout in seconds. `None` uses default.
    /// Range enforced by `Config::validate()`: 1..=600.
    #[serde(default)]
    pub read_timeout: Option<u64>,
    /// Idle keep-alive socket eviction timeout in seconds. `None` uses default;
    /// `Some(0)` disables eviction. Range enforced by `Config::validate()`: 0..=3600.
    #[serde(default)]
    pub pool_idle_timeout: Option<u64>,
```

In `impl Default for AppSettings { fn default() -> Self { Self { ... } } }` (around line 186-214), add to the field initializer list:

```rust
            socket_timeout: None,
            read_timeout: None,
            pool_idle_timeout: None,
```

In the existing `test_default_settings()` (around line 320), add three asserts before the trailing `embed_subtitles` check:

```rust
        assert!(settings.socket_timeout.is_none());
        assert!(settings.read_timeout.is_none());
        assert!(settings.pool_idle_timeout.is_none());
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rdlp-desktop test_default_timeout_fields_are_none test_timeout_fields_round_trip_json test_legacy_settings_json_without_timeout_fields_loads test_default_settings`
Expected: 4 PASS.

Run full crate: `cargo test -p rdlp-desktop`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/rdlp-desktop/src-tauri/src/state/app_settings.rs
git commit -m "feat(desktop): add socket_timeout, read_timeout, pool_idle_timeout to AppSettings"
```

---

## Task 4 — Merge `AppSettings` timeouts into `NetworkOptions` (desktop)

**Files:**
- Modify: `crates/rdlp-desktop/src-tauri/src/commands/download.rs:209-303`

- [ ] **Step 1: Write the failing test**

Find the existing test pattern in `crates/rdlp-desktop/src-tauri/src/commands/download.rs` (around line 395+) — there's a helper function for building a `DownloadRequest` from `(options, settings)`. Mirror its signature.

Append a new test:

```rust
#[test]
fn test_settings_timeouts_propagate_to_network_options() {
    let settings = AppSettings {
        socket_timeout: Some(45),
        read_timeout: Some(120),
        pool_idle_timeout: Some(0),
        ..AppSettings::default()
    };
    let options = DownloadOptions::default();
    let req = build_download_request(&options, &settings, "https://example.com/x").expect("build");
    assert_eq!(req.network.timeout_secs, Some(45));
    assert_eq!(req.network.read_timeout_secs, Some(120));
    assert_eq!(req.network.pool_idle_timeout_secs, Some(0));
}

#[test]
fn test_default_settings_leave_timeouts_unset_in_network_options() {
    let settings = AppSettings::default();
    let options = DownloadOptions::default();
    let req = build_download_request(&options, &settings, "https://example.com/x").expect("build");
    assert!(req.network.timeout_secs.is_none());
    assert!(req.network.read_timeout_secs.is_none());
    assert!(req.network.pool_idle_timeout_secs.is_none());
}
```

If the in-tree helper is named differently (e.g. `build_request_for_test`), adjust the call site. Read the test module first.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p rdlp-desktop test_settings_timeouts_propagate test_default_settings_leave_timeouts`
Expected: FAIL — `NetworkOptions { timeout_secs, read_timeout_secs, pool_idle_timeout_secs }` are not currently set from `settings`.

- [ ] **Step 3: Wire the merge**

In `crates/rdlp-desktop/src-tauri/src/commands/download.rs`, locate the `NetworkOptions { ... }` literal inside the `DownloadRequest { ... network: NetworkOptions { ... } ... }` construction (around line 297-303). Replace it with:

```rust
        network: NetworkOptions {
            cookies_from_browser,
            cookies_file,
            rate_limit,
            proxy,
            timeout_secs: settings.socket_timeout,
            read_timeout_secs: settings.read_timeout,
            pool_idle_timeout_secs: settings.pool_idle_timeout,
            ..NetworkOptions::default()
        },
```

(There is no per-request override for these — they come exclusively from `AppSettings`. If a future need arises to override per-job from `DownloadOptions`, the same `options.X.or(settings.X)` pattern used for `proxy` should be applied; out of scope here.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p rdlp-desktop test_settings_timeouts_propagate test_default_settings_leave_timeouts`
Expected: 2 PASS.

Run full crate: `cargo test -p rdlp-desktop`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/rdlp-desktop/src-tauri/src/commands/download.rs
git commit -m "feat(desktop): plumb settings timeouts into NetworkOptions"
```

---

## Task 5 — Mirror timeout fields in TypeScript `AppSettings`

**Files:**
- Modify: `crates/rdlp-desktop/src/types/index.ts` (around lines 380-390 — the `AppSettings` interface)

- [ ] **Step 1: Read the existing AppSettings TS interface**

Run: `grep -n "interface AppSettings\|^}" crates/rdlp-desktop/src/types/index.ts | head -10`
Note the exact field ordering convention (snake_case vs camelCase). Per the existing `proxy: string | null;` and `rate_limit: string | null;` fields, AppSettings uses **snake_case** in TS (it doesn't carry `serde(rename_all = "camelCase")` in Rust).

- [ ] **Step 2: Add three fields**

In `crates/rdlp-desktop/src/types/index.ts`, find the `AppSettings` interface and add (immediately after `rate_limit: string | null;`):

```ts
    socket_timeout: number | null;
    read_timeout: number | null;
    pool_idle_timeout: number | null;
```

- [ ] **Step 3: Type-check**

Run from `crates/rdlp-desktop/`: `npx tsc --noEmit`
Expected: clean (any failures here are usually consumers of `AppSettings.default` factories — fix by adding the new fields wherever `AppSettings` literals appear).

- [ ] **Step 4: Commit**

```bash
git add crates/rdlp-desktop/src/types/index.ts
git commit -m "feat(desktop/ts): mirror timeout fields in AppSettings TS type"
```

---

## Task 6 — Zod schema for network timeout inputs

**Files:**
- Create: `crates/rdlp-desktop/src/views/settings/networkSchema.ts`
- Create: `crates/rdlp-desktop/src/views/settings/networkSchema.test.ts`

- [ ] **Step 1: Write the failing tests first**

Create `crates/rdlp-desktop/src/views/settings/networkSchema.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import {
    socketTimeoutSchema,
    readTimeoutSchema,
    poolIdleTimeoutSchema,
    formStateToPoolIdleTimeout,
    poolIdleTimeoutToFormState,
} from "./networkSchema";

describe("socketTimeoutSchema", () => {
    it("accepts an empty string as null (use default)", () => {
        expect(socketTimeoutSchema.parse("")).toBeNull();
    });
    it("accepts integer 1..=300", () => {
        expect(socketTimeoutSchema.parse("30")).toBe(30);
        expect(socketTimeoutSchema.parse("300")).toBe(300);
    });
    it("rejects 0 (validate.rs requires >=1)", () => {
        expect(() => socketTimeoutSchema.parse("0")).toThrow();
    });
    it("rejects values above 300", () => {
        expect(() => socketTimeoutSchema.parse("301")).toThrow();
    });
    it("rejects negative numbers", () => {
        expect(() => socketTimeoutSchema.parse("-1")).toThrow();
    });
    it("rejects non-integers", () => {
        expect(() => socketTimeoutSchema.parse("3.5")).toThrow();
    });
    it("rejects garbage", () => {
        expect(() => socketTimeoutSchema.parse("abc")).toThrow();
    });
});

describe("readTimeoutSchema", () => {
    it("accepts integer up to 600", () => {
        expect(readTimeoutSchema.parse("600")).toBe(600);
    });
    it("rejects above 600", () => {
        expect(() => readTimeoutSchema.parse("601")).toThrow();
    });
    it("rejects 0", () => {
        expect(() => readTimeoutSchema.parse("0")).toThrow();
    });
});

describe("poolIdleTimeoutSchema (numeric input only — does not see the checkbox)", () => {
    it("accepts integer 1..=3600", () => {
        expect(poolIdleTimeoutSchema.parse("90")).toBe(90);
        expect(poolIdleTimeoutSchema.parse("3600")).toBe(3600);
    });
    it("rejects 0 with the use-checkbox hint (the checkbox produces 0, not the input)", () => {
        expect(() => poolIdleTimeoutSchema.parse("0")).toThrow(/checkbox/i);
    });
    it("rejects above 3600", () => {
        expect(() => poolIdleTimeoutSchema.parse("3601")).toThrow();
    });
    it("accepts empty string as null", () => {
        expect(poolIdleTimeoutSchema.parse("")).toBeNull();
    });
});

describe("pool-idle form mapping", () => {
    it("checkbox off → 0 (sentinel)", () => {
        expect(formStateToPoolIdleTimeout({ evictIdle: false, secondsInput: "90" })).toBe(0);
        expect(formStateToPoolIdleTimeout({ evictIdle: false, secondsInput: "" })).toBe(0);
    });
    it("checkbox on + numeric input → that integer", () => {
        expect(formStateToPoolIdleTimeout({ evictIdle: true, secondsInput: "120" })).toBe(120);
    });
    it("checkbox on + empty input → null (use default)", () => {
        expect(formStateToPoolIdleTimeout({ evictIdle: true, secondsInput: "" })).toBeNull();
    });
    it("hydrate: 0 → checkbox off, numeric stays empty", () => {
        expect(poolIdleTimeoutToFormState(0)).toEqual({ evictIdle: false, secondsInput: "" });
    });
    it("hydrate: positive → checkbox on, numeric populated", () => {
        expect(poolIdleTimeoutToFormState(90)).toEqual({ evictIdle: true, secondsInput: "90" });
    });
    it("hydrate: null → checkbox on, numeric stays empty", () => {
        expect(poolIdleTimeoutToFormState(null)).toEqual({ evictIdle: true, secondsInput: "" });
    });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run from `crates/rdlp-desktop/`: `npm run test -- networkSchema`
Expected: FAIL — module not found.

- [ ] **Step 3: Create the schema module**

Create `crates/rdlp-desktop/src/views/settings/networkSchema.ts`:

```ts
import { z } from "zod";

const intInRange = (min: number, max: number, label: string) =>
    z.preprocess(
        (v) => (v === "" || v === null || v === undefined ? null : Number(v)),
        z.union([
            z.null(),
            z.number({ invalid_type_error: `${label} must be a number` })
                .int(`${label} must be an integer`)
                .min(min, `${label} must be ≥ ${min}`)
                .max(max, `${label} must be ≤ ${max}`),
        ]),
    );

export const socketTimeoutSchema = intInRange(1, 300, "Connection timeout");
export const readTimeoutSchema = intInRange(1, 600, "Read timeout");

// Numeric input alone — the 0-sentinel is owned by the checkbox.
// Reject 0 explicitly so users who type 0 see a hint to use the checkbox.
export const poolIdleTimeoutSchema = z.preprocess(
    (v) => (v === "" || v === null || v === undefined ? null : Number(v)),
    z.union([
        z.null(),
        z.number({ invalid_type_error: "Idle timeout must be a number" })
            .int("Idle timeout must be an integer")
            .refine((n) => n !== 0, {
                message: "Use the checkbox to keep connections alive forever",
            })
            .min(1, "Idle timeout must be ≥ 1")
            .max(3600, "Idle timeout must be ≤ 3600"),
    ]),
);

export interface PoolIdleFormState {
    evictIdle: boolean;
    secondsInput: string;
}

export function formStateToPoolIdleTimeout(state: PoolIdleFormState): number | null {
    if (!state.evictIdle) return 0; // sentinel: disable eviction
    if (state.secondsInput.trim() === "") return null; // use default
    return Number(state.secondsInput);
}

export function poolIdleTimeoutToFormState(value: number | null): PoolIdleFormState {
    if (value === 0) return { evictIdle: false, secondsInput: "" };
    if (value === null) return { evictIdle: true, secondsInput: "" };
    return { evictIdle: true, secondsInput: String(value) };
}
```

Note on idiom: the spec called for `.nullable()` instead of `z.union([z.null(), ...])`. However `.nullable()` does not compose cleanly with the `.refine()` step on `poolIdleTimeoutSchema` (it would apply `refine` to the nullable wrapper, not the underlying number). The `z.union` form keeps the three schemas symmetric. This is functionally equivalent in zod 3.25.

- [ ] **Step 4: Run schema tests to verify they pass**

Run from `crates/rdlp-desktop/`: `npm run test -- networkSchema`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rdlp-desktop/src/views/settings/networkSchema.ts crates/rdlp-desktop/src/views/settings/networkSchema.test.ts
git commit -m "feat(desktop/ui): zod schema for network timeout inputs"
```

---

## Task 7 — `NetworkSection` UI: add three controls

**Files:**
- Modify: `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx`
- Create: `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.test.tsx`

- [ ] **Step 1: Verify Jolly Checkbox availability**

Run: `ls crates/rdlp-desktop/src/components/ui/checkbox.tsx 2>/dev/null && echo present || echo missing`
If missing, run from `crates/rdlp-desktop/`: `npx shadcn@latest add @jolly/checkbox` and verify `components/ui/checkbox.tsx` is created with React Aria imports.

If the project's shadcn `Checkbox` from `@/components/ui/checkbox` is already React-Aria-based, skip this step. Confirm by reading the file's imports — Jolly UI imports start with `react-aria-components`.

- [ ] **Step 2: Write the failing component tests**

Create `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { NetworkSection } from "./NetworkSection";
import type { AppSettings } from "@/types";

const baseDraft: AppSettings = {
    output_dir: ".",
    default_remux: null,
    default_extract_audio: null,
    default_subtitle_format: null,
    default_subtitle_langs: [],
    embed_thumbnail: true,
    embed_metadata: false,
    verbose: false,
    default_search_provider: null,
    normalize_audio: false,
    loudnorm: false,
    loudnorm_preset: null,
    loudnorm_target_i: null,
    loudnorm_target_tp: null,
    loudnorm_target_lra: null,
    loudnorm_dynamic: false,
    loudnorm_precompress: false,
    normalize_boost: false,
    normalize_boost_db: null,
    write_thumbnail: false,
    audio_gain_target: null,
    cookies_from_browser: null,
    cookies_file: null,
    proxy: null,
    rate_limit: null,
    output_template: null,
    embed_subtitles: false,
    socket_timeout: null,
    read_timeout: null,
    pool_idle_timeout: null,
} as AppSettings;

describe("NetworkSection — timeout controls", () => {
    it("renders three timeout inputs with associated labels", () => {
        render(<NetworkSection draft={baseDraft} onChange={vi.fn()} />);
        expect(screen.getByLabelText(/connection timeout/i)).toBeInTheDocument();
        expect(screen.getByLabelText(/read timeout/i)).toBeInTheDocument();
        expect(screen.getByLabelText(/evict idle/i)).toBeInTheDocument();
    });

    it("typing in connection timeout updates draft", () => {
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} onChange={onChange} />);
        const input = screen.getByLabelText(/connection timeout/i) as HTMLInputElement;
        fireEvent.change(input, { target: { value: "45" } });
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: 45 });
    });

    it("typing empty string in connection timeout sets null", () => {
        const draft = { ...baseDraft, socket_timeout: 30 };
        const onChange = vi.fn();
        render(<NetworkSection draft={draft} onChange={onChange} />);
        const input = screen.getByLabelText(/connection timeout/i) as HTMLInputElement;
        fireEvent.change(input, { target: { value: "" } });
        expect(onChange).toHaveBeenCalledWith({ socket_timeout: null });
    });

    it("checkbox unchecked → pool_idle_timeout = 0", () => {
        const draft = { ...baseDraft, pool_idle_timeout: 90 };
        const onChange = vi.fn();
        render(<NetworkSection draft={draft} onChange={onChange} />);
        const checkbox = screen.getByLabelText(/evict idle/i);
        fireEvent.click(checkbox);
        expect(onChange).toHaveBeenCalledWith({ pool_idle_timeout: 0 });
    });

    it("checkbox unchecked disables the numeric input (aria + DOM)", () => {
        const draft = { ...baseDraft, pool_idle_timeout: 0 };
        render(<NetworkSection draft={draft} onChange={vi.fn()} />);
        const numeric = screen.getByLabelText(/idle.*seconds|seconds.*idle/i) as HTMLInputElement;
        expect(numeric).toBeDisabled();
    });

    it("invalid input shows zod error inline", () => {
        const onChange = vi.fn();
        render(<NetworkSection draft={baseDraft} onChange={onChange} />);
        const input = screen.getByLabelText(/connection timeout/i) as HTMLInputElement;
        fireEvent.change(input, { target: { value: "9999" } });
        // The component MUST render an inline error and MUST NOT propagate the invalid value.
        expect(screen.getByText(/must be ≤ 300/i)).toBeInTheDocument();
        // The last onChange should NOT have set socket_timeout: 9999.
        const calls = onChange.mock.calls;
        expect(calls.every(([arg]) => arg.socket_timeout !== 9999)).toBe(true);
    });
});
```

- [ ] **Step 3: Run tests to verify they fail**

Run from `crates/rdlp-desktop/`: `npm run test -- NetworkSection`
Expected: FAIL — controls don't exist yet.

- [ ] **Step 4: Implement the controls**

Replace `crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx` with:

```tsx
import { useState } from "react";
import { Globe, KeyRound } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectTrigger, SelectValue, SelectItem, SelectPopover, SelectListBox } from "@/components/ui/select";
import {
    socketTimeoutSchema,
    readTimeoutSchema,
    poolIdleTimeoutSchema,
    formStateToPoolIdleTimeout,
    poolIdleTimeoutToFormState,
} from "@/views/settings/networkSchema";
import type { AppSettings } from "@/types";

const NONE_KEY = "none";

interface Props {
    draft: AppSettings;
    onChange: (update: Partial<AppSettings>) => void;
}

interface TimeoutFieldProps {
    id: string;
    label: string;
    helper: string;
    value: number | null;
    placeholder: string;
    schema: typeof socketTimeoutSchema;
    onCommit: (next: number | null) => void;
    disabled?: boolean;
}

function TimeoutField({ id, label, helper, value, placeholder, schema, onCommit, disabled }: TimeoutFieldProps) {
    const [raw, setRaw] = useState<string>(value === null ? "" : String(value));
    const [error, setError] = useState<string | null>(null);

    const handleChange = (next: string) => {
        setRaw(next);
        const result = schema.safeParse(next);
        if (!result.success) {
            setError(result.error.errors[0]?.message ?? "Invalid value");
            return;
        }
        setError(null);
        onCommit(result.data as number | null);
    };

    return (
        <div>
            <Label htmlFor={id} className="settings-label">
                {label}
            </Label>
            <div className="flex items-center gap-1">
                <Input
                    id={id}
                    type="number"
                    inputMode="numeric"
                    min={0}
                    placeholder={placeholder}
                    value={raw}
                    onChange={(e) => handleChange(e.target.value)}
                    aria-describedby={`${id}-help`}
                    aria-invalid={error !== null}
                    disabled={disabled}
                    className="font-mono text-xs"
                />
                <span className="text-xs text-muted-foreground">s</span>
            </div>
            <p id={`${id}-help`} className="text-xs text-muted-foreground mt-1">
                {error ?? helper}
            </p>
        </div>
    );
}

export function NetworkSection({ draft, onChange }: Props) {
    const poolIdleForm = poolIdleTimeoutToFormState(draft.pool_idle_timeout);

    const handleEvictToggle = (next: boolean) => {
        onChange({
            pool_idle_timeout: formStateToPoolIdleTimeout({
                evictIdle: next,
                secondsInput: poolIdleForm.secondsInput,
            }),
        });
    };

    return (
        <>
            <section id="settings-network" aria-labelledby="settings-network-heading" className="settings-panel">
                <h3 id="settings-network-heading" className="settings-panel-title">
                    <Globe className="size-3.5" />
                    Network
                </h3>
                <div className="grid grid-cols-2 gap-x-4 gap-y-3">
                    <div>
                        <Label htmlFor="proxy" className="settings-label">Proxy</Label>
                        <Input
                            id="proxy"
                            type="text"
                            placeholder="http://proxy:8080"
                            value={draft.proxy ?? ""}
                            onChange={(e) => onChange({ proxy: e.target.value || null })}
                            className="font-mono text-xs"
                        />
                    </div>
                    <div>
                        <Label htmlFor="rate-limit" className="settings-label">Rate Limit</Label>
                        <Input
                            id="rate-limit"
                            type="text"
                            placeholder="500K, 2M"
                            value={draft.rate_limit ?? ""}
                            onChange={(e) => onChange({ rate_limit: e.target.value || null })}
                            className="font-mono text-xs"
                        />
                    </div>
                    <TimeoutField
                        id="socket-timeout"
                        label="Connection Timeout"
                        helper="Time to establish a connection to the server."
                        value={draft.socket_timeout}
                        placeholder="30"
                        schema={socketTimeoutSchema}
                        onCommit={(v) => onChange({ socket_timeout: v })}
                    />
                    <TimeoutField
                        id="read-timeout"
                        label="Read Timeout"
                        helper="Maximum gap between bytes during a download."
                        value={draft.read_timeout}
                        placeholder="60"
                        schema={readTimeoutSchema}
                        onCommit={(v) => onChange({ read_timeout: v })}
                    />
                    <div className="col-span-2">
                        <div className="flex items-center gap-2">
                            <Checkbox
                                id="evict-idle"
                                isSelected={poolIdleForm.evictIdle}
                                onChange={handleEvictToggle}
                                aria-controls="pool-idle-timeout"
                            />
                            <Label htmlFor="evict-idle" className="settings-label !mb-0">
                                Evict idle connections after
                            </Label>
                            <Input
                                id="pool-idle-timeout"
                                type="number"
                                inputMode="numeric"
                                min={1}
                                placeholder="90"
                                value={poolIdleForm.secondsInput}
                                onChange={(e) => {
                                    const result = poolIdleTimeoutSchema.safeParse(e.target.value);
                                    if (result.success) {
                                        onChange({
                                            pool_idle_timeout: formStateToPoolIdleTimeout({
                                                evictIdle: poolIdleForm.evictIdle,
                                                secondsInput: e.target.value,
                                            }),
                                        });
                                    }
                                }}
                                disabled={!poolIdleForm.evictIdle}
                                aria-describedby="evict-idle-help"
                                className="font-mono text-xs w-20"
                            />
                            <span className="text-xs text-muted-foreground">s</span>
                        </div>
                        <p id="evict-idle-help" className="text-xs text-muted-foreground mt-1">
                            When off, idle keep-alive connections are kept until the OS closes them.
                        </p>
                    </div>
                </div>
            </section>

            <section id="settings-cookies" aria-labelledby="settings-cookies-heading" className="settings-panel">
                <h3 id="settings-cookies-heading" className="settings-panel-title">
                    <KeyRound className="size-3.5" />
                    Cookies
                </h3>
                <div className="grid grid-cols-2 gap-x-4 gap-y-3">
                    <div>
                        <Label className="settings-label">Browser</Label>
                        <Select
                            selectedKey={draft.cookies_from_browser ?? NONE_KEY}
                            onSelectionChange={(key) => {
                                const k = String(key);
                                onChange({ cookies_from_browser: k === NONE_KEY ? null : k });
                            }}
                        >
                            <SelectTrigger className="w-full text-sm">
                                <SelectValue />
                            </SelectTrigger>
                            <SelectPopover>
                                <SelectListBox>
                                    <SelectItem id={NONE_KEY}>None</SelectItem>
                                    <SelectItem id="chrome">Chrome</SelectItem>
                                    <SelectItem id="firefox">Firefox</SelectItem>
                                </SelectListBox>
                            </SelectPopover>
                        </Select>
                    </div>
                    <div>
                        <Label htmlFor="cookies-file" className="settings-label">
                            Cookie File (Netscape)
                        </Label>
                        <Input
                            id="cookies-file"
                            type="text"
                            placeholder="/path/to/cookies.txt"
                            value={draft.cookies_file ?? ""}
                            onChange={(e) => onChange({ cookies_file: e.target.value || null })}
                            className="font-mono text-xs"
                        />
                    </div>
                </div>
            </section>
        </>
    );
}
```

Notes:
- The Jolly UI `Checkbox` API uses `isSelected` / `onChange(boolean)`. If the in-tree `@/components/ui/checkbox` exposes a different prop shape, adjust accordingly. Read `components/ui/checkbox.tsx` first.
- The `TimeoutField` keeps a local `raw` string so the user can type intermediate values without `onChange` firing per keystroke; commits only when `safeParse` succeeds.
- `aria-describedby` and `aria-controls` per the spec's a11y section.

- [ ] **Step 5: Run component tests to verify they pass**

Run from `crates/rdlp-desktop/`: `npm run test -- NetworkSection`
Expected: all PASS.

If any fail, the most likely cause is the `Checkbox` API mismatch (step 4 note). Read `components/ui/checkbox.tsx` and adjust.

- [ ] **Step 6: Type-check the whole frontend**

Run from `crates/rdlp-desktop/`: `npx tsc --noEmit`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/rdlp-desktop/src/views/settings/sections/NetworkSection.tsx crates/rdlp-desktop/src/views/settings/sections/NetworkSection.test.tsx
git commit -m "feat(desktop/ui): expose timeout controls in Settings → Network (#278)"
```

---

## Task 8 — Cypress a11y regression check

**Files:** none modified — runs the existing suite.

- [ ] **Step 1: Start the dev server**

Run from `crates/rdlp-desktop/`: `npm run dev`
Wait for the URL to be reported.

- [ ] **Step 2: Run a11y suite in another shell**

Run from `crates/rdlp-desktop/`: `npm run test:e2e -- --spec cypress/e2e/a11y.cy.ts`
Expected: all PASS. WCAG 2.1 AA across all 5 views, including Settings.

If failures appear on Settings only, they MUST be fixed inline — never added to `cypress/support/a11y-allowlist.ts` (rule: allowlist stays empty).

- [ ] **Step 3: Stop dev server, commit nothing.** (No artifacts produced.)

---

## Task 9 — Manual smoke verification

**Files:** none modified.

- [ ] **Step 1: CLI smoke**

```bash
cargo run -p rdlp-cli -- --help | grep -E '\-\-(socket|read|pool-idle)-timeout'
```
Expected: 3 lines, one per flag.

```bash
cargo run -p rdlp-cli -- --socket-timeout 0 https://example.com/x 2>&1 | grep -i 'must be 1..=300'
```
Expected: validation error from `Config::validate`.

```bash
cargo run -p rdlp-cli -- --pool-idle-timeout 0 --pool-idle-timeout 0 --read-timeout 90 --socket-timeout 30 https://example.com/x
```
Expected: passes config validation (will fail at extraction, but that's downstream).

- [ ] **Step 2: Desktop smoke**

```bash
cd crates/rdlp-desktop && npm run tauri dev
```

In the running app:
- Open Settings → Network. Verify the three new controls render in the same 2-col rhythm as Proxy / Rate Limit.
- Type `0` into Connection Timeout. Inline error appears: "must be ≥ 1".
- Type `400` into Connection Timeout. Inline error: "must be ≤ 300".
- Type `30`. Error clears, value persists. Save settings, reopen — value reloaded.
- Toggle "Evict idle connections after" off. Numeric input greys out and is `disabled`. Save. Reload — checkbox state preserved.
- Tab through the section with keyboard only. Focus rings visible on every control. The disabled numeric input is skipped in tab order when checkbox is off.

- [ ] **Step 3: Verification gate**

```bash
cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings && cargo test
```
Expected: all green.

```bash
cd crates/rdlp-desktop && npx tsc --noEmit && npm run test
```
Expected: all green.

- [ ] **Step 4: No commit.** Smoke verification produces no artifacts.

---

## Pre-push gate (per rules)

Per `~/.claude/rules/rust-format-before-push.md` and `~/.claude/rules/mandatory-pre-push-review.md`:

1. `cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings && cargo test`
2. From `crates/rdlp-desktop/`: `npx tsc --noEmit && npm run test`
3. Dispatch `security-reviewer` over `develop..HEAD` diff. Fix any HIGH/Critical findings before push.
4. Dispatch `pr-review-toolkit:code-reviewer` over the same diff. Address findings.
5. Only then `git push -u origin feature/network-timeout-cli-and-settings`.
6. `gh pr create` with `Closes #278` in the body.

---

## Self-review

**Spec coverage:**
- CLI flags (3): Task 2 ✓
- AppSettings fields (3): Task 3 ✓
- AppSettings → NetworkOptions merge: Task 4 ✓
- NetworkOptions → Config merge: Task 1 ✓
- TS type mirror: Task 5 ✓
- Zod schema: Task 6 ✓
- UI controls: Task 7 ✓
- A11y: Task 7 (component) + Task 8 (suite) ✓
- Validation flow stays in `Config::validate()`: Tasks 2/4 explicitly do not duplicate it ✓

**Placeholder scan:** none. Each step contains exact code or exact commands. Where in-tree shapes need to be confirmed (e.g. `merge_args_into_config` signature, Jolly Checkbox prop names), the step says "Read X first" rather than guessing.

**Type consistency:**
- Rust: `socket_timeout` / `read_timeout` / `pool_idle_timeout` are all `Option<u64>` everywhere.
- TS: `socket_timeout: number | null` matches Rust serde for `Option<u64>` without rename_all.
- `NetworkOptions` field names: `timeout_secs` (existing, preserved), `read_timeout_secs`, `pool_idle_timeout_secs`. All `Option<u64>`.
- Zod schemas all return `number | null`.

**Scope:** single PR, single sprint, ~10-15 minutes per task. No spec section unaddressed.
