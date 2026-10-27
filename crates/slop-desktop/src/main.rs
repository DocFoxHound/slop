// SPDX-License-Identifier: AGPL-3.0-or-later
//! The SLAPorSLOP desktop app: a Tauri shell around the shared web UI (`web/`).
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
    link::{Link, ModRequest, ModResult, Outcome},
    signin,
    slop_manifest::{self, Platform, Signed},
};
use tauri::{AppHandle, Emitter, Manager, State};

/// The SLAPorSLOP site and API. Set SLOP_API_URL when building a release; the default is the local
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
    /// The app's identifier from tauri.conf.json: "dev.ironpoint.slop", or the test server's
    /// "Slop Dev" app's own (deploy/desktop-config.js). The OS keychain entries (the sign-in and
    /// chat keys) are named after it, so the two apps never read each other's.
    keychain_service: String,
    /// Slop Link: running games ask for mods here (DESIGN §9).
    link: Link,
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

/// Starts a browser sign-in: asks SLAPorSLOP for an approval page and tries to open it in the system
/// browser. The UI then calls `sign_in_finish` to wait for the approval.
#[tauri::command]
async fn sign_in_start(desktop: State<'_, Desktop>) -> CommandResult<SignInStarted> {
    let device = format!(
        "SLAPorSLOP for {}",
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

/// Asks SLAPorSLOP whether a newer version exists. `None` when up to date, offline, or in debug builds.
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

/// The name people see in window titles and the tray: "SLAPorSLOP", or "SLAPorSLOP Dev" for the
/// test server's app. The bundle's product name (`package_info().name`) stays "Slop" or "Slop Dev"
/// after the rename: Tauri's Windows installer keys the install folder, the uninstall entry, the
/// saved install location and the Start menu shortcut on it, and the Linux packages are named
/// after it, so a new one would install a second copy instead of updating this one.
fn display_name(product_name: &str) -> String {
    match product_name.strip_prefix("Slop") {
        Some(rest) => format!("SLAPorSLOP{rest}"),
        None => product_name.to_string(),
    }
}
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
    .title(format!(
        "Sign in to {}",
        display_name(&app.package_info().name)
    ))
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
/// SLAPorSLOP keeps running so it can tell you about updates, and the tray icon brings it back.
/// Minimizing still goes to the taskbar. Elsewhere, closing quits: the tray needs an
/// appindicator library on Linux, which not every distro has.
#[cfg(windows)]
fn make_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    // "SLAPorSLOP", or "SLAPorSLOP Dev" for the test server's app (deploy/desktop-config.js).
    let name = display_name(&app.package_info().name);
    let open = MenuItem::with_id(app, "open", format!("Open {name}"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", format!("Quit {name}"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("slop")
        .tooltip(&name)
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
                "SLAPorSLOP didn't answer at launch; opening the store with the saved sign-in"
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
        message: "SLAPorSLOP can't install apps on this system".into(),
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
    /// What it can't protect against on this desktop (X11), to show the player.
    limits: Option<String>,
    settings: slop_client::sandbox::SandboxSettings,
}

#[tauri::command]
fn sandbox_status(desktop: State<'_, Desktop>) -> SandboxStatus {
    let available = SANDBOX.get_or_init(slop_client::sandbox::available);
    SandboxStatus {
        available: available.is_ok(),
        reason: available.clone().err(),
        limits: slop_client::sandbox::limits(),
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

/// A shared runtime an app needs (slop-manifest runtimes.rs).
#[derive(Serialize)]
struct NeededRuntime {
    id: &'static str,
    name: &'static str,
    vendor: &'static str,
}

/// The shared runtimes an installed app needs and this computer doesn't have.
#[tauri::command]
fn missing_runtimes(
    desktop: State<'_, Desktop>,
    slug: String,
) -> CommandResult<Vec<NeededRuntime>> {
    Ok(desktop
        .library
        .missing_runtimes(&slug)?
        .into_iter()
        .map(|r| NeededRuntime {
            id: r.id,
            name: r.name,
            vendor: r.vendor,
        })
        .collect())
}

/// Installs them, after the player agreed: from each vendor, checked against its signature.
#[tauri::command]
async fn install_runtimes(desktop: State<'_, Desktop>, slug: String) -> CommandResult<()> {
    Ok(desktop.library.install_runtimes(&slug).await?)
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
    /// Things in the game's mods folder that aren't mods from SLAPorSLOP, moved out.
    unapproved_mods: Vec<String>,
    /// What save sync did before the start, when it's on (slop-client save_sync.rs).
    save_sync: Option<slop_client::save_sync::Outcome>,
}

/// A game's request for mods, still waiting for the player (also sent as `mod-request`).
#[tauri::command]
fn mod_request(desktop: State<'_, Desktop>, id: String) -> Option<ModRequest> {
    desktop.link.waiting(&id)
}

/// The player's answer to a game's request for mods. On yes, each mod is fetched, checked
/// against SLAPorSLOP's signature, and added if it's an approved mod for that game; the game is told
/// how each went and where its mods are now.
#[tauri::command]
async fn answer_mod_request(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    id: String,
    accept: bool,
) -> CommandResult<()> {
    let Some(request) = desktop.link.waiting(&id) else {
        return Err(CommandError {
            code: "not_found",
            message: "the game stopped waiting for an answer".into(),
        });
    };
    if !accept {
        desktop.link.finish(&id, Outcome::Declined);
        return Ok(());
    }
    let mut results = Vec::new();
    for wanted in &request.mods {
        let result = add_requested_mod(&app, &desktop, &request.game, wanted).await;
        results.push(result.unwrap_or_else(|e| {
            tracing::warn!(r#mod = %wanted.slug, error = %e.message, "couldn't add a requested mod");
            ModResult {
                slug: wanted.slug.clone(),
                result: "unavailable",
                version: None,
            }
        }));
    }
    let revocations = desktop.library.revocations_for_launch(&desktop.api).await;
    let outcome = match desktop
        .library
        .sync_mods(&request.game, revocations.as_ref())
    {
        Ok((folders, _)) => Outcome::Done {
            mods: results,
            folders: folders
                .iter()
                .map(|f| f.to_string_lossy().into_owned())
                .collect(),
        },
        Err(e) => Outcome::Failed {
            message: e.to_string(),
        },
    };
    desktop.link.finish(&id, outcome);
    let _ = app.emit("mods-changed", &request.game);
    Ok(())
}

async fn add_requested_mod(
    app: &AppHandle,
    desktop: &Desktop,
    game: &str,
    wanted: &slop_client::link::Wanted,
) -> CommandResult<ModResult> {
    let signed: Signed = desktop
        .api
        .get_json(&format!("/v1/apps/{}/manifest", urlencode(&wanted.slug)))
        .await?;
    let manifest = desktop.library.verify_manifest(&signed)?;
    // Only an approved mod for this very game.
    if manifest.app_slug != wanted.slug
        || manifest.mod_for.as_ref().map(|g| g.app_slug.as_str()) != Some(game)
    {
        return Ok(ModResult {
            slug: wanted.slug.clone(),
            result: "unavailable",
            version: None,
        });
    }
    let already = desktop
        .library
        .installed_mods(game)
        .into_iter()
        .any(|m| m.slug == wanted.slug && m.release_id == manifest.release_id);
    if !already {
        let slug = wanted.slug.clone();
        desktop
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
            .await?;
    }
    let differs = wanted
        .version
        .as_ref()
        .is_some_and(|v| *v != manifest.version);
    Ok(ModResult {
        slug: wanted.slug.clone(),
        result: if differs {
            "version_differs"
        } else if already {
            "already_added"
        } else {
            "added"
        },
        version: Some(manifest.version),
    })
}

#[derive(Clone, Serialize)]
struct AppCrashed {
    slug: String,
    code: Option<i32>,
    log: String,
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
    // Slop Link: where the game asks for mods while it runs; closed when it exits.
    let link_dir = desktop.link.open(&slug).map_err(Error::from)?;
    // Saves another computer made, before the app starts (never over this one's changes).
    let save_sync = sync_saves(&app, &desktop, &slug, SyncStep::Pull, false).await;
    let launch = desktop
        .library
        .prepare_launch(&slug, revocations.as_ref(), sandbox, Some(&link_dir))
        .inspect_err(|_| desktop.link.close(&link_dir))?;
    let launched = Launched {
        sandboxed: launch.sandboxed,
        quarantined: launch.quarantined,
        copy_skipped: launch.copy_skipped,
        unapproved_mods: launch.unapproved_mods,
        save_sync,
    };
    let mut command = launch.command;
    let link = desktop.link.clone();
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
    // The last lines it writes, offered for a bug report if it crashes (slop-client output.rs).
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| {
        link.close(&link_dir);
        CommandError {
            code: "launch",
            message: format!("couldn't start the app: {e}"),
        }
    })?;
    // Known while it runs, for the in-game overlay (overlay.rs); reaped when it exits.
    let pid = child.id();
    let tail = slop_client::output::Tail::default();
    if let Some(out) = child.stdout.take() {
        tail.follow(out);
    }
    if let Some(err) = child.stderr.take() {
        tail.follow(err);
    }
    let running = app.state::<overlay::Running>();
    running.0.lock().unwrap().insert(pid, slug.clone());
    let app = app.clone();
    let sandboxed = launched.sandboxed;
    let started = std::time::Instant::now();
    std::thread::spawn(move || {
        let status = child.wait();
        link.close(&link_dir);
        app.state::<overlay::Running>()
            .0
            .lock()
            .unwrap()
            .remove(&pid);
        // Closed with an error right after starting in the box: probably doesn't work there.
        // Send changed saves to the player's other computers.
        let sync_app = app.clone();
        let sync_slug = slug.clone();
        tauri::async_runtime::spawn(async move {
            let desktop = sync_app.state::<Desktop>();
            if let Some(outcome) =
                sync_saves(&sync_app, &desktop, &sync_slug, SyncStep::Push, false).await
            {
                let _ = sync_app.emit(
                    "saves-synced",
                    serde_json::json!({ "slug": sync_slug, "result": outcome }),
                );
            }
        });
        let failed = status.as_ref().is_ok_and(|s| !s.success());
        if sandboxed && started.elapsed() < Duration::from_secs(15) && failed {
            let _ = app.emit("sandbox-failed", SandboxFailed { slug: slug.clone() });
        }
        // Closed with an error: offer a bug report with its last output (the player sees it,
        // edits it or leaves it out before anything is sent).
        if failed {
            // Let the readers catch the last lines.
            std::thread::sleep(Duration::from_millis(200));
            let _ = app.emit(
                "app-crashed",
                AppCrashed {
                    slug,
                    code: status.ok().and_then(|s| s.code()),
                    log: tail.text(slop_client::output::home_dir().as_deref()),
                },
            );
        }
    });
    Ok(launched)
}

// ---------------------------------------------------------------------------------------------
// Save sync: saves follow the player between their computers, end-to-end encrypted with a key
// from their chat backup key (slop-client save_sync.rs).

#[derive(Clone, Copy)]
enum SyncStep {
    Pull,
    Push,
}

/// One sync step for an app, when this computer is signed in, has the key and sync is on.
/// Errors are logged, never shown as a failed launch: the saves on this computer stay as they are.
async fn sync_saves(
    app: &AppHandle,
    desktop: &Desktop,
    slug: &str,
    step: SyncStep,
    force: bool,
) -> Option<slop_client::save_sync::Outcome> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    if !desktop.api.has_session() {
        return None;
    }
    let (saves, copies, scratch, state_path) = desktop.library.save_sync_paths(slug).ok()?;
    let _one_at_a_time = LOCK.lock().await;
    let mut state = slop_client::save_sync::State::load(&state_path);
    if state.off {
        return None;
    }
    let key = chat_for(app, desktop).await.ok()?.save_sync_key().await?;
    let result = match step {
        SyncStep::Pull => {
            slop_client::save_sync::pull(
                &desktop.api,
                &key,
                slug,
                &saves,
                &copies,
                &scratch,
                &mut state,
                force,
            )
            .await
        }
        SyncStep::Push => {
            slop_client::save_sync::push(&desktop.api, &key, slug, &saves, &mut state, force).await
        }
    };
    let _ = state.save(&state_path);
    match result {
        Ok(outcome) => Some(outcome),
        Err(e) => {
            tracing::warn!(error = %e, slug, "save sync");
            None
        }
    }
}

#[derive(Serialize)]
struct SaveSyncStatus {
    /// Signed in, with the key on this computer (chat set up here, backups on).
    available: bool,
    on: bool,
}

#[tauri::command]
async fn save_sync_status(
    app: AppHandle,
    desktop: State<'_, Desktop>,
) -> CommandResult<SaveSyncStatus> {
    let (_, _, _, state_path) = desktop.library.save_sync_paths("x")?;
    let on = !slop_client::save_sync::State::load(&state_path).off;
    let available = desktop.api.has_session()
        && match chat_for(&app, &desktop).await {
            Ok(chat) => chat.save_sync_key().await.is_some(),
            Err(_) => false,
        };
    Ok(SaveSyncStatus { available, on })
}

#[tauri::command]
fn set_save_sync(desktop: State<'_, Desktop>, on: bool) -> CommandResult<()> {
    let (_, _, _, state_path) = desktop.library.save_sync_paths("x")?;
    let mut state = slop_client::save_sync::State::load(&state_path);
    state.off = !on;
    state.save(&state_path).map_err(Error::from)?;
    Ok(())
}

/// Both computers changed an app's saves: keep this computer's (`here`) or the other's.
#[tauri::command]
async fn resolve_save_conflict(
    app: AppHandle,
    desktop: State<'_, Desktop>,
    slug: String,
    keep: String,
) -> CommandResult<Option<slop_client::save_sync::Outcome>> {
    let step = if keep == "here" {
        SyncStep::Push
    } else {
        SyncStep::Pull
    };
    Ok(sync_saves(&app, &desktop, &slug, step, true).await)
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
    /// Sets chat up on this device, named after the computer unless the page names it.
    Setup {
        #[serde(default)]
        name: Option<String>,
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

/// A desktop notification for new messages while SLAPorSLOP isn't in front (DESIGN §19). Muted chats
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
        &KeyringSecrets::new(&desktop.keychain_service),
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
        ChatAction::Setup { name } => {
            let name = name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(slop_client::chat::device_name::this_device_name);
            to_value(chat.setup(&name).await?)
        }
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

/// The apps SLAPorSLOP started that are still running: they're not updated until they close.
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
    // Sandbox mode on Windows: a copy of the app sets up the box and starts the game in it
    // (slop_client::sandbox::launcher_main), before anything else runs.
    #[cfg(windows)]
    {
        let mut args = std::env::args().skip(1);
        if args.next().as_deref() == Some("--slop-sandbox") {
            let spec = args.next().unwrap_or_default();
            std::process::exit(slop_client::sandbox::launcher_main(&spec));
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "slop_desktop=info,slop_client=info".into()),
        )
        .init();
    tauri::Builder::default()
        // A second launch (e.g. from the Start menu while SLAPorSLOP is in the tray) brings back the
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
            let keychain_service = app.config().identifier.clone();
            let api = ApiClient::new(base, Arc::new(KeyringTokenStore::new(&keychain_service)))?;
            let root = app.path().app_data_dir()?.join("library");
            let chat_dir = app.path().app_data_dir()?.join("chat");
            // Plain-http downloads only in debug builds pointed at a local API.
            let allow_http = cfg!(debug_assertions) && API_URL.starts_with("http://localhost");
            // Unsigned builds too, for the same local-development case, or when this build was
            // made for a private test setup (SECURITY §5.3 otherwise).
            let library = Library::new(root.clone(), trusted_keys())?
                .allow_http_downloads(allow_http)
                .require_maker_signature(!allow_http && !allow_unsigned());
            // A game asking for mods: the player answers in the main window.
            let handle = app.handle().clone();
            let link = Link::start(root.join("link"), move |request: &ModRequest| {
                let _ = handle.emit("mod-request", request);
                bring_back(&handle);
                use tauri_plugin_notification::NotificationExt;
                let _ = handle
                    .notification()
                    .builder()
                    .title("A game wants to add mods")
                    .body("Open SLAPorSLOP to see them and say yes or no.")
                    .show();
            })?;
            app.manage(Desktop {
                api,
                library,
                link,
                platform: Platform::current(),
                pending_sign_in: tokio::sync::Mutex::new(None),
                update: tokio::sync::Mutex::new(None),
                chat: tokio::sync::Mutex::new(None),
                chat_dir,
                keychain_service,
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
            missing_runtimes,
            install_runtimes,
            play_choices,
            set_play_choice,
            mod_request,
            answer_mod_request,
            launch_app,
            run_installer,
            save_sync_status,
            set_save_sync,
            resolve_save_conflict,
            chat,
            overlay_hide,
            overlay_setting,
            auto_update_setting,
            running_apps,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the SLAPorSLOP desktop app");
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn windows_and_the_tray_say_slaporslop() {
        assert_eq!(display_name("Slop"), "SLAPorSLOP");
        assert_eq!(display_name("Slop Dev"), "SLAPorSLOP Dev");
        assert_eq!(display_name("SLAPorSLOP"), "SLAPorSLOP");
    }
}
