fn main() {
    // Every command the UI may call is listed here and granted in capabilities/default.json, so
    // nothing else on the Rust side is reachable from the page (SECURITY §7).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "api_request",
            "session_status",
            "sign_in_start",
            "sign_in_finish",
            "sign_out",
            "show_login",
            "continue_signed_out",
            "update_status",
            "install_update",
            "platform_info",
            "installed_apps",
            "install_app",
            "uninstall_app",
            "launch_app",
            "run_installer",
            "chat",
            "overlay_hide",
            "overlay_setting",
            "auto_update_setting",
            "running_apps",
            "installed_mods",
            "install_mod",
            "uninstall_mod",
        ]),
    ))
    .expect("failed to run tauri-build");
}
