//! Tauri build script.
//!
//! Calls [`tauri_build::build`] to generate Tauri's compile-time
//! context (capabilities, plugin permissions, manifest embedding).

fn main() {
    tauri_build::build();
}
