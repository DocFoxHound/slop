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

/// Set to `true` at build time to accept builds without a maker signature: only for private test
/// setups whose repositories can't make GitHub attestations (see docs/TESTING.md).
fn allow_unsigned() -> bool {
    option_env!("SLOP_ALLOW_UNSIGNED") == Some("true")
}

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
    /// A browser sign-in waiting for approval (see `sign_in_start`).
    pending_sign_in: tokio::sync::Mutex<Option<signin::PendingSignIn>>,
    /// A newer version, once one has been found.
    update: tokio::sync::Mutex<Option<UpdateInfo>>,
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
        Self {
            code,
            message: err.to_string(),
        }
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
        _ => Err(CommandError {
            code: "invalid",
            message: format!("unsupported method {name}"),
        }),
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
    Ok(desktop
        .api
        .request(method(&method_name)?, &path, body)
        .await?)
}

#[derive(Serialize)]
struct SessionStatus {
    signed_in: bool,
}

#[tauri::command]
fn session_status(desktop: State<'_, Desktop>) -> SessionStatus {
    SessionStatus {
        signed_in: desktop.api.has_session(),
    }
}

#[derive(Serialize)]
struct SignInStarted {
    /// The approval page. Shown to the user too, in case the browser didn't open.
    approve_url: String,
    browser_opened: bool,
}

/// Starts a browser sign-in: asks Slop for an approval page and tries to open it in the system
/// browser. The UI then calls `sign_in_finish` to wait for the approval.
#[tauri::command]
async fn sign_in_start(desktop: State<'_, Desktop>) -> CommandResult<SignInStarted> {
    let device = format!(
        "Slop for {}",
        desktop.platform.map_or("desktop", |p| match p {
            Platform::Windows => "Windows",
            Platform::Mac => "macOS",
            Platform::Linux => "Linux",
            Platform::Android => "Android",
        })
    );
    let pending = signin::begin(&desktop.api, &device).await?;
    let approve_url = pending.approve_url.clone();
    let browser_opened = match tauri_plugin_opener::open_url(&approve_url, None::<&str>) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "couldn't open the browser for sign-in");
            false
        }
    };
    // Starting again replaces (and so cancels) any earlier sign-in.
    *desktop.pending_sign_in.lock().await = Some(pending);
    Ok(SignInStarted {
        approve_url,
        browser_opened,
    })
}

/// Waits (up to 10 minutes) for the user to approve this device in the browser.
#[tauri::command]
async fn sign_in_finish(app: AppHandle, desktop: State<'_, Desktop>) -> CommandResult<()> {
    let pending = desktop
        .pending_sign_in
        .lock()
        .await
        .take()
        .ok_or(CommandError {
            code: "invalid",
            message: "No sign-in in progress.".into(),
        })?;
    pending
        .finish(&desktop.api, Duration::from_secs(600))
        .await?;
    let _ = app.emit("session-changed", ());
    show_main(&app);
    Ok(())
}

/// Signs out, then closes the store and shows the sign-in window, like Steam.
#[tauri::command]
async fn sign_out(app: AppHandle, desktop: State<'_, Desktop>) -> CommandResult<()> {
    desktop.api.sign_out().await?;
    let _ = app.emit("session-changed", ());
    open_login(&app);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.hide();
    }
    Ok(())
}

/// Opens the sign-in window (from "Sign in" in the store).
#[tauri::command]
fn show_login(app: AppHandle) {
    open_login(&app);
}

/// "Browse without signing in": the store opens signed out.
#[tauri::command]
fn continue_signed_out(app: AppHandle) {
    show_main(&app);
}

// ---------------------------------------------------------------------------------------------
// Updates: never forced. The app checks quietly and tells the UI, which shows a dismissible notice;
// the update is only installed when the player chooses it (SECURITY §7: signed and verified).

/// How often to look for a new version while the app is open.
const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(3 * 60 * 60);

#[derive(Clone, Serialize)]
struct UpdateInfo {
    version: String,
    current_version: String,
    notes: Option<String>,
}

/// Asks Slop whether a newer version exists. `None` when up to date, offline, or in debug builds.
async fn check_for_update(app: &AppHandle) -> Option<tauri_plugin_updater::Update> {
    use tauri_plugin_updater::UpdaterExt;
    if cfg!(debug_assertions) {
        return None;
    }
    let check = async { app.updater()?.check().await };
    match tokio::time::timeout(Duration::from_secs(20), check).await {
        Ok(Ok(update)) => update,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "update check failed");
            None
        }
        Err(_) => {
            tracing::warn!("update check timed out");
            None
        }
    }
}

/// Runs for the life of the app: checks soon after launch, then every few hours, and tells the
/// windows when a newer version is available.
async fn watch_for_updates(app: AppHandle) {
    tokio::time::sleep(Duration::from_secs(5)).await;
    loop {
        if let Some(update) = check_for_update(&app).await {
            let info = UpdateInfo {
                version: update.version.clone(),
                current_version: update.current_version.clone(),
                notes: update.body.clone(),
            };
            tracing::info!(version = %info.version, "update available");
            *app.state::<Desktop>().update.lock().await = Some(info.clone());
            let _ = app.emit("update-available", info);
        }
        tokio::time::sleep(UPDATE_CHECK_EVERY).await;
    }
}

/// The newest version found, if any (for windows that open after the check).
#[tauri::command]
async fn update_status(desktop: State<'_, Desktop>) -> CommandResult<Option<UpdateInfo>> {
    Ok(desktop.update.lock().await.clone())
}

/// The player chose to update: download, verify, install and restart.
#[tauri::command]
async fn install_update(app: AppHandle) -> CommandResult<()> {
    let update = check_for_update(&app).await.ok_or(CommandError {
        code: "update",
        message: "No update is available right now.".into(),
    })?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| CommandError {
            code: "update",
            message: format!("couldn't install the update: {e}"),
        })?;
    // On Windows the installer has already closed the app; elsewhere, start the new version.
    app.restart();
}

// ---------------------------------------------------------------------------------------------
// Windows: a small sign-in window first, then the store (docs: like Steam's launcher).

const LOGIN_WINDOW: &str = "login";

fn open_login(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = tauri::WebviewWindowBuilder::new(
        app,
        LOGIN_WINDOW,
        tauri::WebviewUrl::App("login.html".into()),
    )
    .title("Sign in to Slop")
    .decorations(false)
    .shadow(true)
    .inner_size(420.0, 560.0)
    .resizable(false)
    .maximizable(false)
    .center()
    .build();
    if let Err(e) = built {
        tracing::error!(error = %e, "couldn't open the sign-in window; showing the store");
        show_main(app);
    }
}

/// On Windows, closing the store hides it to the notification area (the tray), like Steam:
/// Slop keeps running so it can tell you about updates, and the tray icon brings it back.
/// Minimizing still goes to the taskbar. Elsewhere, closing quits.
const CLOSE_TO_TRAY: bool = cfg!(windows);

fn make_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    let open = MenuItem::with_id(app, "open", "Open Slop", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Slop", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("slop")
        .tooltip("Slop")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => bring_back(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                bring_back(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Shows whichever window is in use: the sign-in window if it's open, otherwise the store.
fn bring_back(app: &AppHandle) {
    let window = app
        .get_webview_window(LOGIN_WINDOW)
        .or_else(|| app.get_webview_window("main"));
    if let Some(w) = window {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn show_main(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
    if let Some(login) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = login.close();
    }
}

/// At launch: skip the sign-in window when a remembered sign-in still works. Offline, open the
/// store anyway so installed games can be played; only a refused sign-in asks again.
async fn startup(app: AppHandle) {
    tauri::async_runtime::spawn(watch_for_updates(app.clone()));
    let api = app.state::<Desktop>().api.clone();
    if !api.has_session() {
        open_login(&app);
        return;
    }
    let check = tokio::time::timeout(
        Duration::from_secs(8),
        api.request(slop_client::Method::GET, "/v1/me", None),
    )
    .await;
    match check {
        Ok(Ok(res)) if res.status == 401 => open_login(&app),
        Ok(Ok(_)) => show_main(&app),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "offline at launch; opening the store with the saved sign-in");
            show_main(&app);
        }
        Err(_) => {
            tracing::warn!(
                "Slop didn't answer at launch; opening the store with the saved sign-in"
            );
            show_main(&app);
        }
    }
}

#[derive(Serialize)]
struct PlatformInfo {
    platform: Option<Platform>,
    can_install: bool,
}

#[tauri::command]
fn platform_info(desktop: State<'_, Desktop>) -> PlatformInfo {
    PlatformInfo {
        platform: desktop.platform,
        can_install: !TRUSTED_KEYS.trim().is_empty(),
    }
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
async fn install_app(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    slug: String,
) -> CommandResult<InstalledApp> {
    let platform = desktop.platform.ok_or_else(|| CommandError {
        code: "install",
        message: "Slop can't install apps on this system".into(),
    })?;
    let signed: Signed = desktop
        .api
        .get_json(&format!("/v1/apps/{}/manifest", urlencode(&slug)))
        .await?;
    let installed = desktop
        .library
        .install(&signed, platform, Some(&desktop.api), |got, total| {
            let _ = app.emit(
                "install-progress",
                Progress {
                    slug: slug.clone(),
                    got,
                    total,
                },
            );
        })
        .await?;
    if desktop.api.has_session() {
        // Best effort: the install works either way.
        let _ = desktop
            .api
            .request(
                slop_client::Method::PUT,
                &format!("/v1/me/library/{}", urlencode(&slug)),
                None,
            )
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
    let mut command = desktop
        .library
        .launch_command(&slug, revocations.as_ref())?;
    // Slop Pulse: a signed-in launch gets a one-time token for the game to ping with
    // (DESIGN §11). Best effort: the game starts either way.
    if desktop.api.has_session() {
        let path = format!("/v1/apps/{}/launch", urlencode(&slug));
        let launched = tokio::time::timeout(
            Duration::from_secs(3),
            desktop.api.request(slop_client::Method::POST, &path, None),
        )
        .await;
        if let Ok(Ok(res)) = launched
            && res.status == 200
        {
            if let Some(token) = res.body["token"].as_str() {
                command.env("SLOP_PULSE_TOKEN", token);
            }
            if let Some(url) = res.body["pulse_url"].as_str() {
                command.env("SLOP_PULSE_URL", url);
            }
            if let Some(seconds) = res.body["heartbeat_seconds"].as_i64() {
                command.env("SLOP_PULSE_INTERVAL", seconds.to_string());
            }
        }
    }
    let mut child = command.spawn().map_err(|e| CommandError {
        code: "launch",
        message: format!("couldn't start the app: {e}"),
    })?;
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
    let path = desktop
        .library
        .installer_path(&slug, revocations.as_ref())?;
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| CommandError {
        code: "launch",
        message: format!("couldn't start the installer: {e}"),
    })
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn trusted_keys() -> Vec<slop_manifest::ed25519_dalek::VerifyingKey> {
    TRUSTED_KEYS
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| {
            slop_manifest::verifying_key_from_base64(k)
                .expect("SLOP_TRUSTED_KEYS holds an invalid key")
        })
        .collect()
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "slop_desktop=info,slop_client=info".into()),
        )
        .init();
    tauri::Builder::default()
        // A second launch (e.g. from the Start menu while Slop is in the tray) brings back the
        // running one instead of starting another. Registered first, as the plugin asks.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            bring_back(app)
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let base = API_URL.parse().expect("SLOP_API_URL is a valid URL");
            let api = ApiClient::new(base, Arc::new(KeyringTokenStore::new(APP_ID)))?;
            let root = app.path().app_data_dir()?.join("library");
            // Plain-http downloads only in debug builds pointed at a local API.
            let allow_http = cfg!(debug_assertions) && API_URL.starts_with("http://localhost");
            // Unsigned builds too, for the same local-development case, or when this build was
            // made for a private test setup (SECURITY §5.3 otherwise).
            let library = Library::new(root, trusted_keys())?
                .allow_http_downloads(allow_http)
                .require_maker_signature(!allow_http && !allow_unsigned());
            app.manage(Desktop {
                api,
                library,
                platform: Platform::current(),
                pending_sign_in: tokio::sync::Mutex::new(None),
                update: tokio::sync::Mutex::new(None),
            });
            if CLOSE_TO_TRAY && let Err(e) = make_tray(app) {
                tracing::error!(error = %e, "couldn't add the tray icon");
            }
            tauri::async_runtime::spawn(startup(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && CLOSE_TO_TRAY
                && window.label() == "main"
                && window.app_handle().tray_by_id("slop").is_some()
            {
                api.prevent_close();
                let _ = window.hide();
                return;
            }
            // Closing the sign-in window while the store is still hidden (at launch) quits, so
            // the app never keeps running with no window.
            if window.label() == LOGIN_WINDOW && matches!(event, tauri::WindowEvent::Destroyed) {
                let app = window.app_handle();
                let store_visible = app
                    .get_webview_window("main")
                    .and_then(|main| main.is_visible().ok())
                    .unwrap_or(false);
                if !store_visible {
                    app.exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            api_request,
            session_status,
            sign_in_start,
            sign_in_finish,
            sign_out,
            show_login,
            continue_signed_out,
            update_status,
            install_update,
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
