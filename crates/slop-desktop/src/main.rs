//! The Slop desktop app: a Tauri shell around the shared web UI (`web/`).
//!
//! The commands here are thin wrappers over `slop-client`, which does the real work (and is
//! tested on its own): API calls with the user's tokens, browser sign-in, and verified installs.
//! The page never sees tokens or touches the file system directly (SECURITY §7).

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod overlay;

use std::{sync::Arc, time::Duration};

use serde::Serialize;
use serde_json::Value;
use slop_client::{
    Error,
    api::{ApiClient, ApiResponse, KeyringTokenStore},
    chat::{Chat, KeyringSecrets, OutgoingFile},
    library::{InstalledApp, InstalledMod, Library},
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
    /// Chat for the signed-in user, opened on first use (DESIGN §19).
    chat: tokio::sync::Mutex<Option<Arc<Chat>>>,
    chat_dir: std::path::PathBuf,
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
            Platform::Linux | Platform::LinuxDeb | Platform::LinuxRpm => "Linux",
            Platform::Android => "Android",
            Platform::Any => "desktop",
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
    // Chat stops syncing; its encrypted state stays for the next sign-in.
    *desktop.chat.lock().await = None;
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
/// The in-game overlay (overlay.rs).
const OVERLAY_WINDOW: &str = "overlay";

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
/// Minimizing still goes to the taskbar. Elsewhere, closing quits: the tray needs an
/// appindicator library on Linux, which not every distro has.
#[cfg(windows)]
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
        Ok(Ok(_)) => {
            show_main(&app);
            // Chat set up on this device: start syncing now, so messages and unread counts
            // arrive without opening the chat page first.
            let desktop = app.state::<Desktop>();
            let has_chat = std::fs::read_dir(&desktop.chat_dir)
                .map(|mut d| {
                    d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".bin")))
                })
                .unwrap_or(false);
            if has_chat && let Err(e) = chat_for(&app, &desktop).await {
                tracing::warn!(error = %e.message, "couldn't start chat");
            }
        }
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
fn uninstall_app(
    desktop: State<'_, Desktop>,
    slug: String,
    delete_saves: Option<bool>,
) -> CommandResult<()> {
    desktop.library.uninstall(&slug)?;
    if delete_saves == Some(true) {
        desktop.library.delete_saves(&slug)?;
    }
    Ok(())
}

// Sandbox mode and save folders (DESIGN §6) ------------------------------------------------

static SANDBOX: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();

#[derive(Serialize)]
struct SandboxStatus {
    /// Whether sandbox mode works on this computer, and why not.
    available: bool,
    reason: Option<String>,
    settings: slop_client::sandbox::SandboxSettings,
}

#[tauri::command]
fn sandbox_status(desktop: State<'_, Desktop>) -> SandboxStatus {
    let available = SANDBOX.get_or_init(slop_client::sandbox::available);
    SandboxStatus {
        available: available.is_ok(),
        reason: available.clone().err(),
        settings: desktop.library.sandbox_settings(),
    }
}

/// The player's choice for one app (`slug`), or "run every app in sandbox mode" (no slug).
#[tauri::command]
fn set_sandbox(
    desktop: State<'_, Desktop>,
    slug: Option<String>,
    on: bool,
    block_network: Option<bool>,
) -> CommandResult<slop_client::sandbox::SandboxSettings> {
    Ok(desktop.library.set_sandbox(|s| match slug {
        Some(slug) => {
            s.apps.insert(
                slug,
                slop_client::sandbox::AppSandbox {
                    on,
                    block_network: on && block_network.unwrap_or(false),
                },
            );
        }
        None => s.all = on,
    })?)
}

#[tauri::command]
fn save_copies(
    desktop: State<'_, Desktop>,
    slug: String,
) -> CommandResult<Vec<slop_client::saves::SaveCopy>> {
    Ok(desktop.library.save_copies(&slug)?)
}

#[tauri::command]
fn restore_saves(desktop: State<'_, Desktop>, slug: String, id: String) -> CommandResult<()> {
    Ok(desktop.library.restore_saves(&slug, &id)?)
}

/// Switching to another build of a game: brings the saves along.
#[tauri::command]
fn copy_saves(desktop: State<'_, Desktop>, from: String, to: String) -> CommandResult<()> {
    Ok(desktop.library.copy_saves(&from, &to)?)
}

/// Bytes in an app's save folder.
#[tauri::command]
fn save_size(desktop: State<'_, Desktop>, slug: String) -> CommandResult<u64> {
    Ok(desktop.library.save_size(&slug)?)
}

/// What each original's Play button starts, when the player picked a community build.
#[tauri::command]
fn play_choices(desktop: State<'_, Desktop>) -> std::collections::BTreeMap<String, String> {
    desktop.library.play_choices()
}

/// Picks the build `app`'s Play button starts; `None` for the original.
#[tauri::command]
fn set_play_choice(
    desktop: State<'_, Desktop>,
    app: String,
    build: Option<String>,
) -> CommandResult<()> {
    Ok(desktop.library.set_play_choice(&app, build.as_deref())?)
}

#[derive(Serialize)]
struct Launched {
    sandboxed: bool,
    /// Programs found in the save folder and moved out before the start.
    quarantined: Vec<String>,
    /// The saves were too big to copy before this sandboxed session.
    copy_skipped: bool,
}

#[derive(Clone, Serialize)]
struct SandboxFailed {
    slug: String,
}

/// The mods added to an installed game (DESIGN §9).
#[tauri::command]
fn installed_mods(desktop: State<'_, Desktop>, game: String) -> Vec<InstalledMod> {
    desktop.library.installed_mods(&game)
}

/// Adds the newest approved release of a mod to its game, which must be installed.
#[tauri::command]
async fn install_mod(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    slug: String,
) -> CommandResult<InstalledMod> {
    let signed: Signed = desktop
        .api
        .get_json(&format!("/v1/apps/{}/manifest", urlencode(&slug)))
        .await?;
    Ok(desktop
        .library
        .install_mod(&signed, Some(&desktop.api), |got, total| {
            let _ = app.emit(
                "install-progress",
                Progress {
                    slug: slug.clone(),
                    got,
                    total,
                },
            );
        })
        .await?)
}

#[tauri::command]
fn uninstall_mod(desktop: State<'_, Desktop>, slug: String) -> CommandResult<()> {
    Ok(desktop.library.uninstall_mod(&slug)?)
}

/// Starts an installed app after checking its signature, revocation and files: in sandbox mode
/// if the player chose it for this app, unless `normal_once` ("Run normally this time").
#[tauri::command]
async fn launch_app(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    slug: String,
    normal_once: Option<bool>,
) -> CommandResult<Launched> {
    let revocations = desktop.library.revocations_for_launch(&desktop.api).await;
    let sandbox = if normal_once == Some(true) {
        None
    } else {
        desktop.library.sandbox_settings().for_app(&slug)
    };
    if sandbox.is_some()
        && let Err(reason) = SANDBOX.get_or_init(slop_client::sandbox::available)
    {
        return Err(CommandError {
            code: "sandbox",
            message: reason.clone(),
        });
    }
    let launch = desktop
        .library
        .prepare_launch(&slug, revocations.as_ref(), sandbox)?;
    let launched = Launched {
        sandboxed: launch.sandboxed,
        quarantined: launch.quarantined,
        copy_skipped: launch.copy_skipped,
    };
    let mut command = launch.command;
    // Slop Pulse: a signed-in launch gets a one-time token for the game to ping with
    // (DESIGN §11). Best effort: the game starts either way.
    if desktop.api.has_session() {
        let platform = if cfg!(windows) { "windows" } else { "linux" };
        let path = format!("/v1/apps/{}/launch?platform={platform}", urlencode(&slug));
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
    // Known while it runs, for the in-game overlay (overlay.rs); reaped when it exits.
    let pid = child.id();
    let running = app.state::<overlay::Running>();
    running.0.lock().unwrap().insert(pid, slug.clone());
    let app = app.clone();
    let sandboxed = launched.sandboxed;
    let started = std::time::Instant::now();
    std::thread::spawn(move || {
        let status = child.wait();
        app.state::<overlay::Running>()
            .0
            .lock()
            .unwrap()
            .remove(&pid);
        // Closed with an error right after starting in the box: probably doesn't work there.
        if sandboxed
            && started.elapsed() < Duration::from_secs(15)
            && status.is_ok_and(|s| !s.success())
        {
            let _ = app.emit("sandbox-failed", SandboxFailed { slug });
        }
    });
    Ok(launched)
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

// ---------------------------------------------------------------------------------------------
// Chat: end-to-end encrypted in slop-client; the page only sees what it shows (SECURITY §8.5).

/// What the chat page can ask for. One command, so the page's reach stays one entry point.
#[derive(serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ChatAction {
    Status,
    Setup {
        name: String,
    },
    Approve {
        device: uuid::Uuid,
        code: String,
    },
    StartOver,
    RemoveDevice {
        device: uuid::Uuid,
    },
    Conversations,
    Messages {
        conversation: uuid::Uuid,
        before: Option<i64>,
    },
    Send {
        conversation: uuid::Uuid,
        text: String,
        reply_to: Option<uuid::Uuid>,
        #[serde(default)]
        files: Vec<FileIn>,
        link_preview: bool,
    },
    React {
        conversation: uuid::Uuid,
        message: uuid::Uuid,
        emoji: String,
        on: bool,
    },
    Edit {
        conversation: uuid::Uuid,
        message: uuid::Uuid,
        text: String,
    },
    Delete {
        conversation: uuid::Uuid,
        message: uuid::Uuid,
    },
    Rename {
        conversation: uuid::Uuid,
        name: String,
    },
    StartDirect {
        user: String,
    },
    StartGroup {
        users: Vec<String>,
        name: Option<String>,
    },
    StartTeam {
        team: String,
    },
    AddPeople {
        conversation: uuid::Uuid,
        users: Vec<String>,
    },
    Leave {
        conversation: uuid::Uuid,
    },
    RemovePerson {
        conversation: uuid::Uuid,
        user: uuid::Uuid,
    },
    Hide {
        conversation: uuid::Uuid,
    },
    Mute {
        conversation: uuid::Uuid,
        muted: bool,
    },
    MarkRead {
        conversation: uuid::Uuid,
    },
    Attachment {
        conversation: uuid::Uuid,
        message: uuid::Uuid,
        attachment: uuid::Uuid,
    },
    SecurityCodes {
        conversation: uuid::Uuid,
    },
    Report {
        conversation: uuid::Uuid,
        message: uuid::Uuid,
        reason: String,
        details: String,
    },
    SetBackups {
        on: bool,
    },
    Restore {
        code: String,
    },
    Typing {
        conversation: uuid::Uuid,
        typing: bool,
    },
    /// Desktop notifications for new messages: on or off, and whether they show the text.
    Notifications {
        on: bool,
        show_text: bool,
    },
    NotificationSettings,
}

/// Desktop notification settings, kept in the app's data folder.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct NotifySettings {
    on: bool,
    show_text: bool,
}

impl Default for NotifySettings {
    fn default() -> Self {
        Self {
            on: true,
            show_text: true,
        }
    }
}

fn notify_settings_path(desktop: &Desktop) -> std::path::PathBuf {
    desktop.chat_dir.join("notifications.json")
}

fn load_notify_settings(desktop: &Desktop) -> NotifySettings {
    std::fs::read(notify_settings_path(desktop))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// A desktop notification for new messages while Slop isn't in front (DESIGN §19). Muted chats
/// never notify; the text is left out if the user chose that.
fn notify_new_messages(app: &AppHandle, messages: &[slop_client::chat::NewMessage]) {
    use tauri_plugin_notification::NotificationExt;
    if messages.is_empty() {
        return;
    }
    let focused = ["main", OVERLAY_WINDOW].iter().any(|label| {
        app.get_webview_window(label)
            .and_then(|w| Some(w.is_focused().ok()? && w.is_visible().ok()?))
            .unwrap_or(false)
    });
    if focused {
        return;
    }
    let settings = load_notify_settings(&app.state::<Desktop>());
    if !settings.on {
        return;
    }
    // A burst from one chat is one notification.
    let mut by_chat: Vec<(&slop_client::chat::NewMessage, usize)> = Vec::new();
    for m in messages {
        match by_chat
            .iter_mut()
            .find(|(x, _)| x.conversation == m.conversation)
        {
            Some((last, n)) => {
                *last = m;
                *n += 1;
            }
            None => by_chat.push((m, 1)),
        }
    }
    for (m, n) in by_chat {
        let title = if m.title == m.sender {
            m.sender.clone()
        } else {
            format!("{} in {}", m.sender, m.title)
        };
        let body = if !settings.show_text {
            if n == 1 {
                "New message".to_string()
            } else {
                format!("{n} new messages")
            }
        } else if n == 1 {
            m.text.clone()
        } else {
            format!("{} (+{} more)", m.text, n - 1)
        };
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            tracing::warn!(error = %e, "couldn't show a notification");
        }
    }
}

#[derive(serde::Deserialize)]
struct FileIn {
    name: String,
    mime: String,
    /// Base64.
    data: String,
}

/// The signed-in user's chat, opened (and its sync loop started) on first use.
async fn chat_for(app: &AppHandle, desktop: &Desktop) -> CommandResult<Arc<Chat>> {
    let mut slot = desktop.chat.lock().await;
    if let Some(chat) = slot.as_ref() {
        return Ok(chat.clone());
    }
    let me: Value = desktop.api.get_json("/v1/me").await?;
    let user: uuid::Uuid = me["id"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or(CommandError {
            code: "signed_out",
            message: "Sign in to chat.".into(),
        })?;
    let chat = Arc::new(Chat::open(
        desktop.api.clone(),
        desktop.chat_dir.clone(),
        &KeyringSecrets::new(APP_ID),
        user,
    )?);
    *slot = Some(chat.clone());
    tauri::async_runtime::spawn(chat_loop(app.clone(), Arc::downgrade(&chat)));
    Ok(chat)
}

/// Long-polls for new messages while chat is open, and tells the page what changed. Stops when
/// the user signs out (the chat is dropped).
async fn chat_loop(app: AppHandle, chat: std::sync::Weak<Chat>) {
    let mut backoff = 2;
    loop {
        let Some(c) = chat.upgrade() else {
            return;
        };
        let started = std::time::Instant::now();
        match c.sync(25).await {
            Ok(changes) => {
                backoff = 2;
                let changed = changes.list
                    || changes.devices
                    || !changes.conversations.is_empty()
                    || !changes.typing.is_empty();
                notify_new_messages(&app, &changes.new_messages);
                if changed {
                    let _ = app.emit("chat-changed", changes);
                }
                drop(c);
                // A sync that came straight back with nothing (chat not set up or approved on
                // this device yet) waits a little before asking again.
                let pause = if !changed && started.elapsed() < Duration::from_secs(1) {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(300)
                };
                tokio::time::sleep(pause).await;
            }
            Err(e) => {
                drop(c);
                tracing::warn!(error = %e, "chat sync");
                tokio::time::sleep(Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
            }
        }
    }
}

fn to_value<T: Serialize>(v: T) -> CommandResult<Value> {
    serde_json::to_value(v).map_err(|e| CommandError {
        code: "invalid",
        message: e.to_string(),
    })
}

#[tauri::command]
async fn chat(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    action: ChatAction,
) -> CommandResult<Value> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let chat = chat_for(&app, &desktop).await?;
    let ok = || Ok(Value::Null);
    match action {
        ChatAction::Status => to_value(chat.status().await?),
        ChatAction::Setup { name } => to_value(chat.setup(&name).await?),
        ChatAction::Approve { device, code } => {
            chat.approve(device, &code).await?;
            ok()
        }
        ChatAction::StartOver => {
            chat.start_over().await?;
            ok()
        }
        ChatAction::RemoveDevice { device } => {
            chat.remove_device(device).await?;
            ok()
        }
        ChatAction::Conversations => to_value(chat.conversations().await?),
        ChatAction::Messages {
            conversation,
            before,
        } => to_value(chat.messages(conversation, before, 200).await?),
        ChatAction::Send {
            conversation,
            text,
            reply_to,
            files,
            link_preview,
        } => {
            let mut out = Vec::new();
            for f in files {
                out.push(OutgoingFile {
                    name: f.name,
                    mime: f.mime,
                    bytes: b64.decode(f.data).map_err(|_| CommandError {
                        code: "invalid",
                        message: "a file couldn't be read".into(),
                    })?,
                });
            }
            to_value(
                chat.send(conversation, &text, reply_to, out, link_preview)
                    .await?,
            )
        }
        ChatAction::React {
            conversation,
            message,
            emoji,
            on,
        } => {
            chat.react(conversation, message, &emoji, on).await?;
            ok()
        }
        ChatAction::Edit {
            conversation,
            message,
            text,
        } => {
            chat.edit(conversation, message, &text).await?;
            ok()
        }
        ChatAction::Delete {
            conversation,
            message,
        } => {
            chat.delete(conversation, message).await?;
            ok()
        }
        ChatAction::Rename { conversation, name } => {
            chat.rename(conversation, &name).await?;
            ok()
        }
        ChatAction::StartDirect { user } => to_value(chat.start_direct(&user).await?),
        ChatAction::StartGroup { users, name } => {
            to_value(chat.start_group(&users, name.as_deref()).await?)
        }
        ChatAction::StartTeam { team } => to_value(chat.start_team(&team).await?),
        ChatAction::AddPeople {
            conversation,
            users,
        } => {
            chat.add_people(conversation, &users).await?;
            ok()
        }
        ChatAction::Leave { conversation } => {
            chat.leave(conversation).await?;
            ok()
        }
        ChatAction::RemovePerson { conversation, user } => {
            chat.remove_person(conversation, user).await?;
            ok()
        }
        ChatAction::Hide { conversation } => {
            chat.hide(conversation).await?;
            ok()
        }
        ChatAction::Mute {
            conversation,
            muted,
        } => {
            chat.set_muted(conversation, muted).await?;
            ok()
        }
        ChatAction::MarkRead { conversation } => {
            chat.mark_read(conversation).await?;
            ok()
        }
        ChatAction::Attachment {
            conversation,
            message,
            attachment,
        } => {
            let (mime, bytes) = chat.attachment(conversation, message, attachment).await?;
            Ok(serde_json::json!({ "mime": mime, "data": b64.encode(bytes) }))
        }
        ChatAction::SecurityCodes { conversation } => {
            to_value(chat.security_codes(conversation).await?)
        }
        ChatAction::Report {
            conversation,
            message,
            reason,
            details,
        } => Ok(chat
            .report(conversation, message, &reason, &details)
            .await?),
        ChatAction::SetBackups { on } => to_value(chat.set_backups(on).await?),
        ChatAction::Restore { code } => to_value(chat.restore_backup(&code).await?),
        ChatAction::Typing {
            conversation,
            typing,
        } => {
            chat.typing(conversation, typing).await?;
            ok()
        }
        ChatAction::Notifications { on, show_text } => {
            let settings = NotifySettings { on, show_text };
            std::fs::create_dir_all(&desktop.chat_dir).map_err(Error::from)?;
            std::fs::write(
                notify_settings_path(&desktop),
                serde_json::to_vec(&settings).unwrap_or_default(),
            )
            .map_err(Error::from)?;
            to_value(settings)
        }
        ChatAction::NotificationSettings => to_value(load_notify_settings(&desktop)),
    }
}

/// The overlay's Esc or "Back to the game".
#[tauri::command]
fn overlay_hide(app: AppHandle) {
    overlay::hide(&app);
}

/// "Update apps automatically" (DESIGN §6): on unless the player turned it off. The library
/// installs approved updates for apps that aren't running.
#[tauri::command]
fn auto_update_setting(desktop: State<'_, Desktop>, on: Option<bool>) -> CommandResult<bool> {
    let path = desktop.chat_dir.join("auto-update.json");
    if let Some(on) = on {
        std::fs::create_dir_all(&desktop.chat_dir).map_err(Error::from)?;
        std::fs::write(&path, if on { "true" } else { "false" }).map_err(Error::from)?;
    }
    Ok(std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<bool>(&b).ok())
        .unwrap_or(true))
}

/// The apps Slop started that are still running: they're not updated until they close.
#[tauri::command]
fn running_apps(app: AppHandle) -> Vec<String> {
    let mut slugs: Vec<String> = app
        .state::<overlay::Running>()
        .0
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect();
    slugs.sort();
    slugs.dedup();
    slugs
}

/// The in-game overlay switch: reads it, or sets it when `on` is given.
#[tauri::command]
fn overlay_setting(desktop: State<'_, Desktop>, on: Option<bool>) -> CommandResult<bool> {
    if let Some(on) = on {
        overlay::set_enabled(&desktop, on).map_err(Error::from)?;
    }
    Ok(overlay::enabled(&desktop))
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
        .plugin(tauri_plugin_notification::init())
        .plugin(overlay::plugin())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let base = API_URL.parse().expect("SLOP_API_URL is a valid URL");
            let api = ApiClient::new(base, Arc::new(KeyringTokenStore::new(APP_ID)))?;
            let root = app.path().app_data_dir()?.join("library");
            let chat_dir = app.path().app_data_dir()?.join("chat");
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
                chat: tokio::sync::Mutex::new(None),
                chat_dir,
            });
            #[cfg(windows)]
            if let Err(e) = make_tray(app) {
                tracing::error!(error = %e, "couldn't add the tray icon");
            }
            app.manage(overlay::Running::default());
            app.manage(overlay::State::default());
            tauri::async_runtime::spawn(overlay::watch(app.handle().clone()));
            tauri::async_runtime::spawn(startup(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
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
            installed_mods,
            install_mod,
            uninstall_mod,
            sandbox_status,
            set_sandbox,
            save_copies,
            restore_saves,
            copy_saves,
            save_size,
            play_choices,
            set_play_choice,
            launch_app,
            run_installer,
            chat,
            overlay_hide,
            overlay_setting,
            auto_update_setting,
            running_apps,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Slop desktop app");
}
