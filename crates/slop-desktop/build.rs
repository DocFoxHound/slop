fn main() {
    // Every command the UI may call is listed here and granted in capabilities/default.json, so
    // nothing else on the Rust side is reachable from the page (SECURITY §7).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "api_request",
            "session_status",
            "sign_in",
            "sign_out",
            "platform_info",
            "installed_apps",
            "install_app",
            "uninstall_app",
            "launch_app",
            "run_installer",
        ]),
    ))
    .expect("failed to run tauri-build");
}
