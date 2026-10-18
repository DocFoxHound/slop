//! The Slop desktop app: a Tauri shell around the shared web UI (`web/`).
//!
//! Installing, verifying and launching apps (docs/ARCHITECTURE.md §3, §6.3) will live here as
//! Tauri commands. For now the shell shows the store and opens external links in the system
//! browser.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running the Slop desktop app");
}
