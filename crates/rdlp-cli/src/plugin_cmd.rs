//! `rdlp plugin <subcommand>` — plugin management commands.

// Explicit `#[path]` because lib.rs loads this file via `#[path = "plugin_cmd.rs"]`,
// which makes implicit submodule lookup resolve from `src/` rather than `src/plugin_cmd/`.
#[path = "plugin_cmd/build_from_ytdlp.rs"]
mod build_from_ytdlp;
pub use build_from_ytdlp::run as run_build_from_ytdlp;

use anyhow::{Context, Result};
use rdlp_plugin::PluginError;
use rdlp_plugin::manifest::{Manifest, validate_plugin_name};
use rdlp_plugin::trust_store::{IdentityCheck, TrustStore};
use rdlp_types::Config;
use std::path::{Path, PathBuf};

/// Reject path-traversing or otherwise unsafe plugin names BEFORE any
/// `dir.join(name)` / `remove_dir_all` operation. Gives the user a clear
/// error message rather than silently mis-resolving the path.
fn require_valid_name(name: &str) -> Result<()> {
    validate_plugin_name(name).map_err(|e| anyhow::anyhow!("invalid plugin name '{name}': {e}"))
}

/// Return the rdlp config directory (`~/.config/rdlp` on most platforms).
///
/// # Errors
///
/// Returns an error if the platform has no config directory and `HOME` is unset.
pub fn config_path() -> Result<PathBuf> {
    Ok(dirs::config_dir().context("no config dir")?.join("rdlp"))
}

/// Return the path to the plugin trust store file.
///
/// # Errors
///
/// Returns an error if the platform has no config directory.
pub fn trust_store_path() -> Result<PathBuf> {
    Ok(config_path()?.join("plugin-trust.toml"))
}

/// Return the path to the plugin disabled-list file.
///
/// # Errors
///
/// Returns an error if the platform has no config directory.
pub fn disabled_list_path() -> Result<PathBuf> {
    Ok(config_path()?.join("plugin-disabled.toml"))
}

/// How an untrusted plugin gets trusted. The default prompter denies an
/// unknown publisher, so nothing prompts: the operator passes the identity
/// and the loader records it once the signature verifies and the identity
/// is approved on a load — which `plugin list` / `plugin info` never do.
fn trust_hint(identity: &str) -> String {
    format!(
        "not loaded until trusted: run rdlp with `--trust-publisher {identity}` \
         (recorded on the first successful load)"
    )
}

/// How much of the way forward a trust line carries.
#[derive(Clone, Copy)]
enum Hint {
    /// The full `--trust-publisher` command (`info`, `retrust`).
    Inline,
    /// A pointer to `plugin info` (`list`, where the identity is already
    /// on the line).
    SeeInfo,
}

/// One line for the trust column: the signature's verdict, then the
/// recorded identity checked against the one the manifest presents
/// (`identity`), with the way forward. An unverifiable signature gets no
/// trust hint — the loader refuses it before it looks at trust.
fn trust_state(
    verified: Result<&IdentityCheck, &PluginError>,
    name: &str,
    identity: &str,
    hint: Hint,
) -> String {
    match verified {
        Err(e) => format!("SIGNATURE INVALID — {e}; not loaded whatever the trust state"),
        Ok(IdentityCheck::Match) => "TRUSTED".into(),
        Ok(IdentityCheck::NewName) => match hint {
            Hint::Inline => format!("UNTRUSTED — {}", trust_hint(identity)),
            Hint::SeeInfo => format!("UNTRUSTED (see `rdlp plugin info {name}`)"),
        },
        Ok(IdentityCheck::Mismatch { recorded, .. }) => format!(
            "IDENTITY CHANGED — recorded {recorded}; after verifying the publisher, \
             run `rdlp plugin retrust {name}`"
        ),
    }
}

/// The manifest's own publisher identity, and its trust state in `trust`
/// after the same signature check the loader performs over
/// `<plugin_dir>/plugin.wasm`.
fn identity_and_state(
    m: &Manifest,
    plugin_dir: &Path,
    trust: &TrustStore,
    hint: Hint,
) -> (String, String) {
    let identity = m.signature.identity_string();
    #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
    let verified = std::fs::read(plugin_dir.join("plugin.wasm"))
        .map_err(|e| PluginError::Internal(format!("read plugin.wasm: {e}")))
        .and_then(|wasm| rdlp_plugin::signature::verify(m, &wasm));
    let check = trust.check_identity_match(&m.name, &identity);
    let state = trust_state(verified.as_ref().map(|()| &check), &m.name, &identity, hint);
    (identity, state)
}

/// `rdlp plugin list` — list all installed plugins with their trust state.
///
/// # Errors
///
/// Returns an error if the trust store cannot be opened.
pub fn run_list(config: &Config) -> Result<()> {
    let trust = TrustStore::open(trust_store_path()?)?;
    if config.plugin_directories.is_empty() {
        println!("(no plugin directories configured; set Config::plugin_directories)");
        return Ok(());
    }
    let mut found = 0usize;
    for dir in &config.plugin_directories {
        #[allow(clippy::disallowed_methods)] // startup/CLI commands — sync I/O is acceptable
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }
            match rdlp_plugin::manifest::parse_manifest_file(&manifest_path) {
                Ok(m) => {
                    found += 1;
                    let (identity, state) = identity_and_state(&m, &path, &trust, Hint::SeeInfo);
                    println!(
                        "{}  v{}  identity={identity}  caps=[{}]  {state}",
                        m.name,
                        m.version,
                        m.capabilities.join(", ")
                    );
                }
                Err(e) => {
                    println!("{}  ERROR: {e}", path.display());
                }
            }
        }
    }
    if found == 0 {
        println!("(no plugins installed)");
    }
    Ok(())
}

/// `rdlp plugin info <name>` — show detailed info for a specific plugin.
///
/// # Errors
///
/// Returns an error if the plugin name is invalid or the manifest cannot be parsed.
pub fn run_info(name: &str, config: &Config) -> Result<()> {
    require_valid_name(name)?;
    if let Some(plugin_dir) = installed_plugin_dir(name, config) {
        let m = rdlp_plugin::manifest::parse_manifest_file(&plugin_dir.join("plugin.toml"))?;
        let trust = TrustStore::open(trust_store_path()?)?;
        let trust_entry = trust.lookup(name);

        println!("Plugin: {}", m.name);
        println!("Version: {}", m.version);
        println!("WIT version: {}", m.wit_version);
        println!("Priority: {}", m.priority);
        println!("Match patterns:");
        for p in &m.matches {
            println!("  - {p}");
        }
        if !m.claims_override.is_empty() {
            println!("Claims override:");
            for h in &m.claims_override {
                println!("  - {h}");
            }
        }
        println!("Capabilities: {}", m.capabilities.join(", "));
        let (identity, state) = identity_and_state(&m, &plugin_dir, &trust, Hint::Inline);
        println!("Identity: {identity}");
        println!("Trust state: {state}");
        if let Some(e) = trust_entry {
            let caps: Vec<_> = e.approved_capabilities.iter().cloned().collect();
            println!("Approved capabilities: {}", caps.join(", "));
        }
        println!("Origin: {}", plugin_dir.display());
        return Ok(());
    }
    anyhow::bail!("plugin '{name}' not found in any configured plugin directory")
}

/// The directory of the installed plugin `name`: the first configured
/// plugin directory holding `<name>/plugin.toml`.
fn installed_plugin_dir(name: &str, config: &Config) -> Option<PathBuf> {
    config
        .plugin_directories
        .iter()
        .map(|dir| dir.join(name))
        .find(|plugin_dir| plugin_dir.join("plugin.toml").exists())
}

/// `rdlp plugin retrust <name>` — clear the recorded identity so the
/// plugin's current identity can be trusted afresh.
///
/// # Errors
///
/// Returns an error if the plugin name is invalid or the trust store
/// cannot be opened or written.
pub fn run_retrust(name: &str, config: &Config) -> Result<()> {
    require_valid_name(name)?;
    let mut trust = TrustStore::open(trust_store_path()?)?;
    if trust.lookup(name).is_some() {
        trust.forget(name)?;
        // Nothing prompts: name the exact flag, with the identity when the
        // manifest is on disk to read it from.
        let how = installed_plugin_dir(name, config)
            .and_then(|dir| {
                rdlp_plugin::manifest::parse_manifest_file(&dir.join("plugin.toml")).ok()
            })
            .map_or_else(
                || {
                    format!(
                        "{} — see `rdlp plugin info {name}`",
                        trust_hint("<identity>")
                    )
                },
                |m| trust_hint(&m.signature.identity_string()),
            );
        println!("Trust forgotten for plugin '{name}'; {how}");
    } else {
        println!("Plugin '{name}' was not in the trust store; nothing to forget.");
    }
    Ok(())
}

/// `rdlp plugin disable <name>` — add the plugin to the disabled list.
///
/// # Errors
///
/// Returns an error if the plugin name is invalid or the disabled list cannot be written.
pub fn run_disable(name: &str) -> Result<()> {
    require_valid_name(name)?;
    let path = disabled_list_path()?;
    // Fail loudly on a corrupted disabled list. Silently treating it as
    // empty would re-enable a previously-blocked plugin — a security
    // regression we explicitly do not want.
    let mut current = read_disabled(&path)
        .with_context(|| format!("read disabled-plugin list at {}", path.display()))?;
    if current.contains(&name.to_string()) {
        println!("Plugin '{name}' is already disabled.");
        return Ok(());
    }
    current.push(name.to_string());
    current.sort();
    write_disabled(&path, &current)?;
    println!("Plugin '{name}' disabled. It will be skipped on next load.");
    Ok(())
}

/// `rdlp plugin enable <name>` — remove the plugin from the disabled list.
///
/// # Errors
///
/// Returns an error if the plugin name is invalid or the disabled list cannot be updated.
pub fn run_enable(name: &str) -> Result<()> {
    require_valid_name(name)?;
    let path = disabled_list_path()?;
    let mut current = read_disabled(&path)
        .with_context(|| format!("read disabled-plugin list at {}", path.display()))?;
    let before = current.len();
    current.retain(|n| n != name);
    if current.len() == before {
        println!("Plugin '{name}' was not disabled.");
        return Ok(());
    }
    write_disabled(&path, &current)?;
    println!("Plugin '{name}' re-enabled.");
    Ok(())
}

/// `rdlp plugin uninstall <name>` — delete the plugin directory and forget its trust entry.
///
/// # Errors
///
/// Returns an error if the plugin name is invalid, the directory cannot be removed,
/// or the trust store cannot be updated.
pub fn run_uninstall(name: &str, config: &Config) -> Result<()> {
    require_valid_name(name)?;
    let mut found = false;
    for dir in &config.plugin_directories {
        let plugin_dir = dir.join(name);
        if plugin_dir.exists() {
            #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
            std::fs::remove_dir_all(&plugin_dir)
                .with_context(|| format!("removing {}", plugin_dir.display()))?;
            println!("Removed plugin directory {}", plugin_dir.display());
            found = true;
        }
    }
    let mut trust = TrustStore::open(trust_store_path()?)?;
    if trust.lookup(name).is_some() {
        trust.forget(name)?;
        println!("Forgot trust entry for '{name}'.");
        found = true;
    }
    if !found {
        anyhow::bail!("plugin '{name}' not installed");
    }
    Ok(())
}

// The disabled-list TOML shape and reader live in `rdlp_plugin::disabled_list`
// so that orchestrator bootstrap (in rdlp-api) can read the same file the
// CLI writes without taking a dependency on rdlp-cli.
use rdlp_plugin::disabled_list::{DisabledList, read_disabled_list as read_disabled};

fn write_disabled(path: &PathBuf, list: &[String]) -> Result<()> {
    let dl = DisabledList {
        disabled: list.to_vec(),
    };
    let s = toml::to_string_pretty(&dl)?;
    if let Some(parent) = path.parent() {
        #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
        std::fs::create_dir_all(parent)?;
    }
    // Atomic write: tmp + rename in the same dir, mirroring TrustStore::persist.
    // Plain `std::fs::write` would corrupt the file on crash mid-write.
    let mut tmp_path = path.clone();
    let tmp_name = format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin-disabled")
    );
    tmp_path.set_file_name(tmp_name);
    #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
    std::fs::write(&tmp_path, s)?;
    #[cfg(unix)]
    {
        // Restrict mode to user-only — mirrors the trust store.
        use std::os::unix::fs::PermissionsExt;
        #[allow(clippy::disallowed_methods)]
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
    }
    #[allow(clippy::disallowed_methods)] // CLI command — sync I/O acceptable
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_plugin::trust_store::IdentityCheck;

    const ID: &str = "ed25519:838282c38f97f6b28c8a8d9a272a415f9f7c2db422d6435d8e8dd7865df8e523";

    /// The identity is the operator's input to `--trust-publisher`; the
    /// hint must carry it verbatim, name the flag, and say when the trust
    /// is actually recorded (on a successful load — not by `plugin list`).
    #[test]
    fn trust_hint_names_the_flag_the_identity_and_when_it_is_recorded() {
        let hint = trust_hint(ID);
        assert!(hint.contains("--trust-publisher"), "{hint}");
        assert!(hint.contains(ID), "{hint}");
        assert!(hint.contains("first successful load"), "{hint}");
    }

    #[test]
    fn trust_state_of_a_new_name_is_untrusted_with_the_hint() {
        let line = trust_state(Ok(&IdentityCheck::NewName), "foo", ID, Hint::Inline);
        assert!(line.starts_with("UNTRUSTED"), "{line}");
        assert!(line.contains("--trust-publisher"), "{line}");
        assert!(line.contains(ID), "the identity reaches the line: {line}");
    }

    /// `list` already prints the identity on the line; its state points
    /// at `info` instead of repeating 64 hex characters inside a hint.
    #[test]
    fn trust_state_for_list_points_at_info_instead_of_repeating_the_identity() {
        let line = trust_state(Ok(&IdentityCheck::NewName), "foo", ID, Hint::SeeInfo);
        assert_eq!(line, "UNTRUSTED (see `rdlp plugin info foo`)");
    }

    /// A manifest the loader would refuse gets no trust hint: trusting an
    /// identity does nothing for a plugin whose signature does not verify.
    #[test]
    fn trust_state_of_an_invalid_signature_names_it_and_gives_no_trust_hint() {
        let line = trust_state(
            Err(&rdlp_plugin::PluginError::SignatureInvalid {
                plugin: "foo".into(),
                reason: "bad sig".into(),
            }),
            "foo",
            ID,
            Hint::Inline,
        );
        assert!(line.starts_with("SIGNATURE INVALID"), "{line}");
        assert!(line.contains("bad sig"), "{line}");
        assert!(!line.contains("--trust-publisher"), "{line}");
    }

    #[test]
    fn trust_state_of_a_match_is_trusted() {
        assert_eq!(
            trust_state(Ok(&IdentityCheck::Match), "foo", ID, Hint::Inline),
            "TRUSTED"
        );
    }

    /// A changed identity names the recorded one and the way out
    /// (`retrust`), never a "will prompt" that the default prompter denies.
    #[test]
    fn trust_state_of_a_mismatch_names_the_recorded_identity_and_retrust() {
        let line = trust_state(
            Ok(&IdentityCheck::Mismatch {
                recorded: "ed25519:0000".into(),
                presented: ID.into(),
            }),
            "foo",
            ID,
            Hint::Inline,
        );
        assert!(line.starts_with("IDENTITY CHANGED"), "{line}");
        assert!(line.contains("ed25519:0000"), "{line}");
        assert!(line.contains("rdlp plugin retrust foo"), "{line}");
        assert!(!line.contains("prompt"), "{line}");
    }
}
