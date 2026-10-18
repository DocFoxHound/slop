//! The Slop desktop app: a Tauri shell around the shared web UI (`web/`).
//!
//! The commands here are thin wrappers over `slop-client`, which does the real work (and is
//! tested on its own): API calls with the user's tokens, browser sign-in, and verified installs.
//! The page never sees tokens or touches the file system directly (SECURITY §7).

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{sync::Arc, time::Duration};

use serde::Serialize;
use serde_json::Value;
use slop_client::{
    Error,
    api::{ApiClient, ApiResponse, KeyringTokenStore},
    library::{InstalledApp, Library},
    signin,
    slop_manifest::{self, Platform, Signed},
};
use tauri::{AppHandle, Emitter, Manager, State};

const APP_ID: &str = "dev.ironpoint.slop";

/// The Slop site and API. Set SLOP_API_URL when building a release; the default is the local
/// development site (Vite on :5173, which proxies /v1 to the API).
const API_URL: &str = match option_env!("SLOP_API_URL") {
    Some(url) => url,
    None => "http://localhost:5173/",
};

/// Public keys whose signatures on release manifests and revocation lists this build trusts
/// (comma-separated base64; see `slop-api keys generate`). Without any, nothing installs.
const TRUSTED_KEYS: &str = match option_env!("SLOP_TRUSTED_KEYS") {
    Some(keys) => keys,
    None => "",
};

struct Desktop {
    api: ApiClient,
    library: Library,
    platform: Option<Platform>,
}

/// Errors as the UI sees them: a short code and a sentence to show.
#[derive(Debug, Serialize)]
struct CommandError {
    code: &'static str,
    message: String,
}

impl From<Error> for CommandError {
    fn from(err: Error) -> Self {
        let code = match &err {
            Error::Network(_) => "network",
            Error::Api { .. } => "api",
            Error::SignedOut => "signed_out",
            Error::Verify(_) => "verify",
            Error::Install(_) => "install",
            Error::Revoked(_) => "revoked",
            Error::Io(_) => "io",
        };
        Self { code, message: err.to_string() }
    }
}

type CommandResult<T> = Result<T, CommandError>;

fn method(name: &str) -> CommandResult<slop_client::Method> {
    match name {
        "GET" => Ok(slop_client::Method::GET),
        "POST" => Ok(slop_client::Method::POST),
        "PUT" => Ok(slop_client::Method::PUT),
        "PATCH" => Ok(slop_client::Method::PATCH),
        "DELETE" => Ok(slop_client::Method::DELETE),
        _ => Err(CommandError { code: "invalid", message: format!("unsupported method {name}") }),
    }
}

/// Calls the API on the user's behalf. Path checks happen in `ApiClient`.
#[tauri::command]
async fn api_request(
    desktop: State<'_, Desktop>,
    method_name: String,
    path: String,
    body: Option<Value>,
) -> CommandResult<ApiResponse> {
    Ok(desktop.api.request(method(&method_name)?, &path, body).await?)
}

#[derive(Serialize)]
struct SessionStatus {
    signed_in: bool,
}

#[tauri::command]
fn session_status(desktop: State<'_, Desktop>) -> SessionStatus {
    SessionStatus { signed_in: desktop.api.has_session() }
}

/// Opens the system browser to sign in, and waits (up to 10 minutes) for it to come back.
#[tauri::command]
async fn sign_in(desktop: State<'_, Desktop>) -> CommandResult<()> {
    let device = format!("Slop for {}", desktop.platform.map_or("desktop", |p| match p {
        Platform::Windows => "Windows",
        Platform::Mac => "macOS",
        Platform::Linux => "Linux",
        Platform::Android => "Android",
    }));
    let pending = signin::begin(&desktop.api, &device).await?;
    tauri_plugin_opener::open_url(&pending.approve_url, None::<&str>)
        .map_err(|e| CommandError { code: "browser", message: format!("couldn't open your browser: {e}") })?;
    pending.finish(&desktop.api, Duration::from_secs(600)).await?;
    Ok(())
}

#[tauri::command]
async fn sign_out(desktop: State<'_, Desktop>) -> CommandResult<()> {
    Ok(desktop.api.sign_out().await?)
}

#[derive(Serialize)]
struct PlatformInfo {
    platform: Option<Platform>,
    can_install: bool,
}

#[tauri::command]
fn platform_info(desktop: State<'_, Desktop>) -> PlatformInfo {
    PlatformInfo { platform: desktop.platform, can_install: !TRUSTED_KEYS.trim().is_empty() }
}

#[tauri::command]
fn installed_apps(desktop: State<'_, Desktop>) -> Vec<InstalledApp> {
    desktop.library.installed()
}

#[derive(Clone, Serialize)]
struct Progress {
    slug: String,
    got: u64,
    total: u64,
}

/// Installs the newest approved release of an app, and adds it to the user's library.
#[tauri::command]
async fn install_app(app: AppHandle, desktop: State<'_, Desktop>, slug: String) -> CommandResult<InstalledApp> {
    let platform = desktop
        .platform
        .ok_or_else(|| CommandError { code: "install", message: "Slop can't install apps on this system".into() })?;
    let signed: Signed = desktop.api.get_json(&format!("/v1/apps/{}/manifest", urlencode(&slug))).await?;
    let installed = desktop
        .library
        .install(&signed, platform, |got, total| {
            let _ = app.emit("install-progress", Progress { slug: slug.clone(), got, total });
        })
        .await?;
    if desktop.api.has_session() {
        // Best effort: the install works either way.
        let _ = desktop
            .api
            .request(slop_client::Method::PUT, &format!("/v1/me/library/{}", urlencode(&slug)), None)
            .await;
    }
    Ok(installed)
}

#[tauri::command]
fn uninstall_app(desktop: State<'_, Desktop>, slug: String) -> CommandResult<()> {
    Ok(desktop.library.uninstall(&slug)?)
}

/// Starts an installed app after checking its signature, revocation and files.
#[tauri::command]
async fn launch_app(desktop: State<'_, Desktop>, slug: String) -> CommandResult<()> {
    let revocations = desktop.library.revocations_for_launch(&desktop.api).await;
    let mut command = desktop.library.launch_command(&slug, revocations.as_ref())?;
    let mut child = command
        .spawn()
        .map_err(|e| CommandError { code: "launch", message: format!("couldn't start the app: {e}") })?;
    // Reap the process when it exits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens a maker's installer with the system, after the user confirmed in the UI.
#[tauri::command]
async fn run_installer(desktop: State<'_, Desktop>, slug: String) -> CommandResult<()> {
    let revocations = desktop.library.revocations_for_launch(&desktop.api).await;
    let path = desktop.library.installer_path(&slug, revocations.as_ref())?;
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|e| CommandError { code: "launch", message: format!("couldn't start the installer: {e}") })
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn trusted_keys() -> Vec<slop_manifest::ed25519_dalek::VerifyingKey> {
    TRUSTED_KEYS
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| slop_manifest::verifying_key_from_base64(k).expect("SLOP_TRUSTED_KEYS holds an invalid key"))
        .collect()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let base = API_URL.parse().expect("SLOP_API_URL is a valid URL");
            let api = ApiClient::new(base, Arc::new(KeyringTokenStore::new(APP_ID)))?;
            let root = app.path().app_data_dir()?.join("library");
            // Plain-http downloads only in debug builds pointed at a local API.
            let allow_http = cfg!(debug_assertions) && API_URL.starts_with("http://localhost");
            let library = Library::new(root, trusted_keys())?.allow_http_downloads(allow_http);
            app.manage(Desktop { api, library, platform: Platform::current() });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api_request,
            session_status,
            sign_in,
            sign_out,
            platform_info,
            installed_apps,
            install_app,
            uninstall_app,
            launch_app,
            run_installer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Slop desktop app");
}
