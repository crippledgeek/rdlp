//! Tauri IPC command handlers.
//!
//! Each submodule groups related commands that are registered with
//! [`tauri::generate_handler!`] in [`crate::run`].

// No outer `///` on the submodules below: each already carries its own `//!`
// doc, and an outer doc merged with it makes rustdoc resolve the inner
// doc's intra-doc links against THIS module's scope instead of the
// submodule's own — "no item named `get_formats` in scope" (#661).
pub mod codecs;
pub mod download;
pub mod formats;
pub mod search;
pub mod settings;
pub mod thumbnail;
