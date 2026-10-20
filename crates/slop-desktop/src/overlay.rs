//! The in-game overlay (DESIGN §19): Shift+Tab over a game launched from Slop brings up a small
//! panel with friends and chats, and Shift+Tab (or Esc) puts the game back.
//!
//! It stays out of the way everywhere else. Shift+Tab is only taken while **a game started from
//! Slop is the window in front and fills its screen**: fullscreen, borderless fullscreen, or a
//! maximized window. Anywhere else (a tool, the desktop, the Slop window itself) the shortcut is
//! released, so Shift+Tab keeps doing what it normally does.
//!
//! The overlay is an ordinary always-on-top window over the game; nothing is injected into the
//! game. Games in exclusive fullscreen switch to the desktop while it's open, the way Alt+Tab
//! would, and come back when it closes. Supported on Windows, and on Linux under X11 (games
//! running through XWayland included).

use std::{collections::HashMap, sync::Mutex, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::{Desktop, OVERLAY_WINDOW};

/// Games started from Slop: process ID → the app's address.
#[derive(Default)]
pub struct Running(pub Mutex<HashMap<u32, String>>);

/// What the overlay needs to put the game back.
#[derive(Default)]
pub struct State {
    /// The shortcut is registered (a game is in front, full screen).
    armed: Mutex<bool>,
    /// The game window the overlay is over, and its screen.
    over: Mutex<Option<Foreground>>,
}

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn contains_center_of(&self, other: &Rect) -> bool {
        let (cx, cy) = (other.x + other.w / 2, other.y + other.h / 2);
        cx >= self.x && cx < self.x + self.w && cy >= self.y && cy < self.y + self.h
    }

    /// Covers `other` (allowing a few pixels for borders a game leaves).
    fn covers(&self, other: &Rect) -> bool {
        const SLACK: i32 = 2;
        self.x <= other.x + SLACK
            && self.y <= other.y + SLACK
            && self.x + self.w >= other.x + other.w - SLACK
            && self.y + self.h >= other.y + other.h - SLACK
    }
}

/// The window in front.
#[derive(Clone, Debug)]
pub struct Foreground {
    /// Its native handle (HWND or X11 window), for giving it focus back.
    pub handle: u64,
    pub pid: u32,
    pub rect: Rect,
    /// The screen it's on, and that screen without the taskbar or panels.
    pub monitor: Rect,
    pub work_area: Rect,
    /// The window manager says it's fullscreen or maximized.
    pub fullscreen_state: bool,
    pub maximized: bool,
}

/// Fullscreen (exclusive or borderless) or a maximized window: the ways games fill a screen.
pub fn fills_screen(f: &Foreground) -> bool {
    f.fullscreen_state || f.maximized || f.rect.covers(&f.monitor) || f.rect.covers(&f.work_area)
}

/// Whether `pid` is one of the games started from Slop, or a process one of them started.
pub fn belongs_to(pid: u32, games: &[u32], parent_of: &dyn Fn(u32) -> Option<u32>) -> bool {
    let mut current = pid;
    for _ in 0..32 {
        if games.contains(&current) {
            return true;
        }
        match parent_of(current) {
            Some(p) if p != current && p > 1 => current = p,
            _ => return false,
        }
    }
    false
}

fn shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::SHIFT), Code::Tab)
}

/// The global shortcut plugin, wired to toggle the overlay.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, sc, event| {
            if *sc == shortcut() && event.state() == ShortcutState::Pressed {
                toggle(app);
            }
        })
        .build()
}

fn overlay_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(OVERLAY_WINDOW) {
        return Some(w);
    }
    tauri::WebviewWindowBuilder::new(
        app,
        OVERLAY_WINDOW,
        tauri::WebviewUrl::App("overlay.html".into()),
    )
    .title("Slop")
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .visible(false)
    .build()
    .inspect_err(|e| tracing::error!(error = %e, "couldn't open the overlay"))
    .ok()
}

pub fn is_open(app: &AppHandle) -> bool {
    app.get_webview_window(OVERLAY_WINDOW)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

#[derive(Clone, Serialize)]
struct Shown {
    /// The game's name.
    game: Option<String>,
}

fn toggle(app: &AppHandle) {
    if is_open(app) {
        hide(app);
        return;
    }
    let state = app.state::<State>();
    let Some(game) = state.over.lock().unwrap().clone() else {
        return;
    };
    let Some(window) = overlay_window(app) else {
        return;
    };
    let m = game.monitor;
    let _ = window.set_position(tauri::PhysicalPosition::new(m.x, m.y));
    let _ = window.set_size(tauri::PhysicalSize::new(
        m.w.max(1) as u32,
        m.h.max(1) as u32,
    ));
    let _ = window.show();
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();
    let desktop = app.state::<Desktop>();
    let slug = app
        .state::<Running>()
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|(pid, _)| belongs_to(game.pid, &[**pid], &platform::parent_of))
        .map(|(_, slug)| slug.clone());
    let title = slug.and_then(|s| {
        desktop
            .library
            .installed()
            .into_iter()
            .find(|a| a.slug == s)
            .map(|a| a.title)
    });
    let _ = app.emit_to(OVERLAY_WINDOW, "overlay-shown", Shown { game: title });
}

/// Closes the overlay and gives the game its focus back.
pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(OVERLAY_WINDOW) {
        let _ = w.hide();
    }
    let game = app.state::<State>().over.lock().unwrap().clone();
    if let Some(g) = game {
        platform::focus(g.handle);
    }
}

fn arm(app: &AppHandle, on: bool) {
    let state = app.state::<State>();
    let mut armed = state.armed.lock().unwrap();
    if *armed == on {
        return;
    }
    let gs = app.global_shortcut();
    let done = if on {
        gs.register(shortcut())
    } else {
        gs.unregister(shortcut())
    };
    match done {
        Ok(()) => *armed = on,
        Err(e) => tracing::warn!(error = %e, on, "overlay shortcut"),
    }
}

/// Whether the overlay is switched on (Account → This device), kept beside the notification
/// settings.
pub fn enabled(desktop: &Desktop) -> bool {
    std::fs::read(desktop.chat_dir.join("overlay.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<bool>(&b).ok())
        .unwrap_or(true)
}

pub fn set_enabled(desktop: &Desktop, on: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(&desktop.chat_dir)?;
    std::fs::write(
        desktop.chat_dir.join("overlay.json"),
        if on { "true" } else { "false" },
    )
}

/// Runs for the life of the app: takes Shift+Tab only while a Slop game fills the screen in
/// front, and lets it go otherwise.
pub async fn watch(app: AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let games: Vec<u32> = app
            .state::<Running>()
            .0
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect();
        let desktop = app.state::<Desktop>();
        if games.is_empty() || !enabled(&desktop) {
            if is_open(&app) {
                hide(&app);
            }
            arm(&app, false);
            continue;
        }
        // While the overlay is open it's in front; keep the shortcut so it closes it.
        if is_open(&app) {
            arm(&app, true);
            continue;
        }
        let front = tokio::task::spawn_blocking(platform::foreground)
            .await
            .ok()
            .flatten();
        let game =
            front.filter(|f| fills_screen(f) && belongs_to(f.pid, &games, &platform::parent_of));
        arm(&app, game.is_some());
        *app.state::<State>().over.lock().unwrap() = game;
    }
}

#[cfg(windows)]
mod platform {
    use super::{Foreground, Rect};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HWND, RECT},
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsZoomed,
            SetForegroundWindow,
        },
    };

    fn rect(r: RECT) -> Rect {
        Rect {
            x: r.left,
            y: r.top,
            w: r.right - r.left,
            h: r.bottom - r.top,
        }
    }

    pub fn foreground() -> Option<Foreground> {
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut r) == 0 {
                return None;
            }
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            Some(Foreground {
                handle: hwnd as u64,
                pid,
                rect: rect(r),
                monitor: rect(info.rcMonitor),
                work_area: rect(info.rcWork),
                fullscreen_state: false,
                maximized: IsZoomed(hwnd) != 0,
            })
        }
    }

    pub fn parent_of(pid: u32) -> Option<u32> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap as isize == -1 {
                return None;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = None;
            if Process32FirstW(snap, &mut entry) != 0 {
                loop {
                    if entry.th32ProcessID == pid {
                        found = Some(entry.th32ParentProcessID);
                        break;
                    }
                    if Process32NextW(snap, &mut entry) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snap);
            found
        }
    }

    pub fn focus(handle: u64) {
        unsafe {
            SetForegroundWindow(handle as HWND);
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Foreground, Rect};
    use x11rb::{
        connection::Connection,
        protocol::{
            randr::ConnectionExt as _,
            xproto::{AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, Window},
        },
    };

    fn atom(conn: &impl Connection, name: &str) -> Option<u32> {
        Some(
            conn.intern_atom(false, name.as_bytes())
                .ok()?
                .reply()
                .ok()?
                .atom,
        )
    }

    fn property32(conn: &impl Connection, window: Window, name: &str, kind: AtomEnum) -> Vec<u32> {
        let Some(prop) = atom(conn, name) else {
            return vec![];
        };
        conn.get_property(false, window, prop, kind, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect()))
            .unwrap_or_default()
    }

    /// The active window per the window manager (_NET_ACTIVE_WINDOW). Under Wayland this sees
    /// games running through XWayland, which is how most games run there.
    pub fn foreground() -> Option<Foreground> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen].root;
        let active = *property32(&conn, root, "_NET_ACTIVE_WINDOW", AtomEnum::WINDOW).first()?;
        if active == 0 {
            return None;
        }
        let pid = *property32(&conn, active, "_NET_WM_PID", AtomEnum::CARDINAL).first()?;
        let geometry = conn.get_geometry(active).ok()?.reply().ok()?;
        let at = conn
            .translate_coordinates(active, root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let rect = Rect {
            x: i32::from(at.dst_x),
            y: i32::from(at.dst_y),
            w: i32::from(geometry.width),
            h: i32::from(geometry.height),
        };
        let states = property32(&conn, active, "_NET_WM_STATE", AtomEnum::ATOM);
        let has = |name: &str| atom(&conn, name).is_some_and(|a| states.contains(&a));
        let fullscreen_state = has("_NET_WM_STATE_FULLSCREEN");
        let maximized = has("_NET_WM_STATE_MAXIMIZED_VERT") && has("_NET_WM_STATE_MAXIMIZED_HORZ");
        let monitors: Vec<Rect> = conn
            .randr_get_monitors(root, true)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| {
                r.monitors
                    .iter()
                    .map(|m| Rect {
                        x: i32::from(m.x),
                        y: i32::from(m.y),
                        w: i32::from(m.width),
                        h: i32::from(m.height),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let root_geometry = conn.get_geometry(root).ok()?.reply().ok()?;
        let monitor = monitors
            .iter()
            .copied()
            .find(|m| m.contains_center_of(&rect))
            .unwrap_or(Rect {
                x: 0,
                y: 0,
                w: i32::from(root_geometry.width),
                h: i32::from(root_geometry.height),
            });
        Some(Foreground {
            handle: u64::from(active),
            pid,
            rect,
            monitor,
            // Panels aren't known per monitor here; a maximized window says so itself.
            work_area: monitor,
            fullscreen_state,
            maximized,
        })
    }

    pub fn parent_of(pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // "pid (name) state ppid …": the name can hold spaces and parentheses.
        let after = &stat[stat.rfind(')')? + 1..];
        after.split_whitespace().nth(1)?.parse().ok()
    }

    pub fn focus(handle: u64) {
        let Ok((conn, screen)) = x11rb::connect(None) else {
            return;
        };
        let root = conn.setup().roots[screen].root;
        let Some(active) = atom(&conn, "_NET_ACTIVE_WINDOW") else {
            return;
        };
        // Source 2: a request from a pager or tool acting for the user.
        let event = ClientMessageEvent::new(32, handle as Window, active, [2, 0, 0, 0, 0]);
        let _ = conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        );
        let _ = conn.flush();
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::Foreground;
    pub fn foreground() -> Option<Foreground> {
        None
    }
    pub fn parent_of(_pid: u32) -> Option<u32> {
        None
    }
    pub fn focus(_handle: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(rect: Rect, maximized: bool) -> Foreground {
        Foreground {
            handle: 1,
            pid: 10,
            rect,
            monitor: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            work_area: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1040,
            },
            fullscreen_state: false,
            maximized,
        }
    }

    #[test]
    fn only_screen_filling_windows_count() {
        // Borderless fullscreen, a maximized window above the taskbar, and a plain window.
        assert!(fills_screen(&window(
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080
            },
            false
        )));
        assert!(fills_screen(&window(
            Rect {
                x: -8,
                y: -8,
                w: 1936,
                h: 1056
            },
            true
        )));
        assert!(!fills_screen(&window(
            Rect {
                x: 200,
                y: 100,
                w: 1280,
                h: 720
            },
            false
        )));
        // On the second screen.
        let mut second = window(
            Rect {
                x: 1920,
                y: 0,
                w: 2560,
                h: 1440,
            },
            false,
        );
        second.monitor = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1440,
        };
        second.work_area = second.monitor;
        assert!(fills_screen(&second));
    }

    #[test]
    fn only_games_started_from_slop_count() {
        // 30 is a launcher Slop started; 31 is the game it started; 40 is something else.
        let parents = |pid: u32| match pid {
            31 => Some(30),
            30 => Some(5),
            40 => Some(5),
            _ => None,
        };
        assert!(belongs_to(31, &[30], &parents));
        assert!(belongs_to(30, &[30], &parents));
        assert!(!belongs_to(40, &[30], &parents));
        assert!(!belongs_to(5, &[30], &parents));
    }
}

/// Against a real X server (run under Xvfb, playing the window manager's part):
/// `DISPLAY=:99 cargo test -p slop-desktop overlay -- --ignored`.
#[cfg(all(test, target_os = "linux"))]
mod x11_tests {
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass},
        wrapper::ConnectionExt as _,
    };

    #[test]
    #[ignore = "needs an X server"]
    fn finds_a_fullscreen_window_in_front() {
        let (conn, screen) = x11rb::connect(None).unwrap();
        let s = &conn.setup().roots[screen];
        let (root, w, h) = (s.root, s.width_in_pixels, s.height_in_pixels);
        let win = conn.generate_id().unwrap();
        conn.create_window(0, win, root, 0, 0, w, h, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new())
            .unwrap();
        conn.map_window(win).unwrap();
        let atom = |n: &str| conn.intern_atom(false, n.as_bytes()).unwrap().reply().unwrap().atom;
        conn.change_property32(PropMode::REPLACE, win, atom("_NET_WM_PID"), AtomEnum::CARDINAL, &[std::process::id()])
            .unwrap();
        conn.change_property32(
            PropMode::REPLACE,
            win,
            atom("_NET_WM_STATE"),
            AtomEnum::ATOM,
            &[atom("_NET_WM_STATE_FULLSCREEN")],
        )
        .unwrap();
        conn.change_property32(PropMode::REPLACE, root, atom("_NET_ACTIVE_WINDOW"), AtomEnum::WINDOW, &[win])
            .unwrap();
        conn.sync().unwrap();
        let f = super::platform::foreground().expect("the window in front");
        assert_eq!(f.pid, std::process::id());
        assert!(f.fullscreen_state);
        assert_eq!((f.rect.w, f.rect.h), (i32::from(w), i32::from(h)));
        assert!(super::fills_screen(&f));
        // Started by this process's parent: a game launched from Slop, or a child of one.
        let parent = super::platform::parent_of(std::process::id()).unwrap();
        assert!(super::belongs_to(f.pid, &[parent], &super::platform::parent_of));
        assert!(!super::belongs_to(f.pid, &[999_999], &super::platform::parent_of));

        // A small window in front instead: not full screen, so Shift+Tab is left alone.
        conn.change_property32(PropMode::REPLACE, win, atom("_NET_WM_STATE"), AtomEnum::ATOM, &[])
            .unwrap();
        conn.configure_window(
            win,
            &x11rb::protocol::xproto::ConfigureWindowAux::new().width(640).height(480).x(100).y(100),
        )
        .unwrap();
        conn.sync().unwrap();
        let f = super::platform::foreground().unwrap();
        assert!(!super::fills_screen(&f), "{f:?}");
    }
}
