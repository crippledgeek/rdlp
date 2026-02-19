//! Entry point for the rdlp desktop application.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    rdlp_desktop::run();
}
