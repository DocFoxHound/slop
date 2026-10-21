//! Sandbox mode (DESIGN §6, SECURITY §6.2): an app started in its own box, where it can read
//! its installed files and mods, write only its save folder, and use the screen, sound, graphics
//! card and controllers. The rest of the computer (documents, browsers, keys, other apps, Slop
//! itself) isn't there. Players choose it per app; Slop suggests it for apps that go online.
//!
//! Linux uses bubblewrap, the sandbox Flatpak is built on. Windows (AppContainer) comes next;
//! until then `available` says no there.
//!
//! It limits what a compromised app can reach. It isn't a guarantee: on X11 desktops any window
//! can read the keyboard, sandboxed or not.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

/// What goes in the box besides the system's own read-only files.
pub struct Sandbox<'a> {
    /// The app's installed files (read-only); the program starts here.
    pub files: &'a Path,
    /// Its mods' folders (read-only).
    pub mods: &'a [PathBuf],
    /// Its save folder: the only place it can write that outlives the session.
    pub saves: &'a Path,
    /// Its cache folder (writable, never copied or kept with saves).
    pub cache: &'a Path,
    /// Whether it may use the network.
    pub network: bool,
}

/// A player's sandbox choice for one app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSandbox {
    pub on: bool,
    /// Only offered for apps whose store page says they don't go online.
    #[serde(default)]
    pub block_network: bool,
}

/// The player's sandbox settings, kept in the library folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSettings {
    /// "Run every app in sandbox mode".
    #[serde(default)]
    pub all: bool,
    /// Per app, once the player chose. An app missing here hasn't been decided.
    #[serde(default)]
    pub apps: std::collections::BTreeMap<String, AppSandbox>,
}

impl SandboxSettings {
    /// Whether this app runs sandboxed, and with the network.
    pub fn for_app(&self, slug: &str) -> Option<AppSandbox> {
        match self.apps.get(slug) {
            Some(app) if app.on => Some(*app),
            Some(_) => None,
            None if self.all => Some(AppSandbox {
                on: true,
                block_network: false,
            }),
            None => None,
        }
    }
}

/// Whether sandbox mode works on this computer; the reason when it doesn't.
pub fn available() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let Some(bwrap) = bwrap_path() else {
            return Err("sandbox mode needs bubblewrap: install the \"bubblewrap\" package".into());
        };
        let out = Command::new(bwrap)
            .args([
                "--unshare-all",
                "--ro-bind",
                "/",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "true",
            ])
            .output()
            .map_err(|e| format!("couldn't run bubblewrap: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "this system doesn't let bubblewrap make a sandbox: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("sandbox mode isn't available on this system yet".into())
    }
}

#[cfg(target_os = "linux")]
fn bwrap_path() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .chain([PathBuf::from("/usr/bin")])
        .map(|d| d.join("bwrap"))
        .find(|p| p.is_file())
}

/// Wraps `program args…` in the sandbox. The environment set on the returned command reaches
/// the app, except for the session secrets removed here.
#[cfg(target_os = "linux")]
pub fn command(sandbox: &Sandbox, program: &Path, args: &[String]) -> std::io::Result<Command> {
    let bwrap = bwrap_path().ok_or_else(|| std::io::Error::other("bubblewrap isn't installed"))?;
    let mut cmd = Command::new(bwrap);
    cmd.args(arguments(sandbox, &Host::current()))
        .arg("--")
        .arg(program)
        .args(args)
        .current_dir(sandbox.files);
    Ok(cmd)
}

#[cfg(not(target_os = "linux"))]
pub fn command(_: &Sandbox, _: &Path, _: &[String]) -> std::io::Result<Command> {
    Err(std::io::Error::other(
        "sandbox mode isn't available on this system yet",
    ))
}

/// What the box needs from the desktop session: where the user's home and the display, sound
/// and device sockets are.
#[derive(Default)]
pub struct Host {
    pub home: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub wayland: Option<OsString>,
    pub x11: bool,
    pub xauthority: Option<PathBuf>,
    /// Top-level names in `/`: (name, symlink target if it's a link).
    pub root: Vec<(String, Option<PathBuf>)>,
    /// Device nodes for graphics cards and controllers: /dev/nvidia*, /dev/hidraw*.
    pub devices: Vec<PathBuf>,
}

impl Host {
    #[cfg(target_os = "linux")]
    pub fn current() -> Self {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        let home = env("HOME").map(PathBuf::from);
        let root = ["bin", "lib", "lib32", "lib64", "sbin"]
            .into_iter()
            .filter_map(|name| {
                let path = Path::new("/").join(name);
                let meta = std::fs::symlink_metadata(&path).ok()?;
                let link = meta
                    .file_type()
                    .is_symlink()
                    .then(|| std::fs::read_link(&path).ok())
                    .flatten();
                Some((name.to_string(), link))
            })
            .collect();
        let devices = std::fs::read_dir("/dev")
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                n.starts_with("nvidia") || n.starts_with("hidraw")
            })
            .map(|e| e.path())
            .collect();
        Self {
            xauthority: env("XAUTHORITY")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".Xauthority"))),
            home,
            runtime_dir: env("XDG_RUNTIME_DIR").map(PathBuf::from),
            wayland: env("WAYLAND_DISPLAY"),
            x11: env("DISPLAY").is_some(),
            root,
            devices,
        }
    }
}

/// Session secrets and buses the app mustn't reach.
const UNSET: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "GPG_AGENT_INFO",
    "GNOME_KEYRING_CONTROL",
    "SESSION_MANAGER",
    "KRB5CCNAME",
];

/// bubblewrap's arguments. Later mounts go on top of earlier ones, so the order matters: the
/// empty home and /tmp come before the app's own folders inside them.
pub fn arguments(s: &Sandbox, host: &Host) -> Vec<OsString> {
    let os = |s: &str| OsString::from(s);
    let bind = |kind: &str, p: &Path| [os(kind), p.into(), p.into()];

    let mut args: Vec<OsString> = vec![
        os("--unshare-all"),
        os("--new-session"),
        os("--cap-drop"),
        os("ALL"),
    ];
    if s.network {
        args.push(os("--share-net"));
    }
    // The system's programs and libraries, read-only. /etc holds no secrets an ordinary
    // user can read, and games need its fonts, certificates and DNS settings.
    args.extend(bind("--ro-bind", Path::new("/usr")));
    args.extend(bind("--ro-bind", Path::new("/etc")));
    args.extend(bind("--ro-bind-try", Path::new("/opt")));
    for (name, link) in &host.root {
        let at = Path::new("/").join(name);
        match link {
            Some(target) => args.extend([os("--symlink"), target.into(), at.into()]),
            None => args.extend(bind("--ro-bind", &at)),
        }
    }
    args.extend([os("--proc"), os("/proc"), os("--dev"), os("/dev")]);
    args.extend([os("--tmpfs"), os("/dev/shm")]);
    // Graphics, sound and controllers. Device permissions still apply: a keyboard the user
    // can't read outside the box can't be read inside it either.
    for dev in ["/dev/dri", "/dev/snd", "/dev/input"] {
        args.extend(bind("--dev-bind-try", Path::new(dev)));
    }
    for dev in &host.devices {
        args.extend(bind("--dev-bind-try", dev));
    }
    args.extend(bind("--ro-bind-try", Path::new("/sys")));
    args.extend([os("--tmpfs"), os("/tmp"), os("--tmpfs"), os("/run")]);
    if let Some(home) = &host.home {
        args.extend([os("--tmpfs"), home.into()]);
    }
    // The desktop session: display and sound sockets only (no session bus).
    if let Some(run) = &host.runtime_dir {
        args.extend([os("--dir"), run.into()]);
        let mut sockets = vec![run.join("pulse/native"), run.join("pipewire-0")];
        if let Some(w) = &host.wayland {
            sockets.push(run.join(w));
        }
        for socket in sockets {
            args.extend(bind("--ro-bind-try", &socket));
        }
    }
    if host.x11 {
        args.extend(bind("--ro-bind-try", Path::new("/tmp/.X11-unix")));
        if let Some(xa) = &host.xauthority {
            args.extend(bind("--ro-bind-try", xa));
        }
    }
    // The app's own folders, last, so they sit on top of the empty home.
    args.extend(bind("--ro-bind", s.files));
    for m in s.mods {
        args.extend(bind("--ro-bind", m));
    }
    args.extend(bind("--bind", s.saves));
    args.extend(bind("--bind", s.cache));
    for name in UNSET {
        args.extend([os("--unsetenv"), os(name)]);
    }
    args.extend([os("--setenv"), os("SLOP_SANDBOX"), os("1")]);
    args.extend([os("--chdir"), s.files.into()]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            home: Some("/home/ann".into()),
            runtime_dir: Some("/run/user/1000".into()),
            wayland: Some("wayland-0".into()),
            x11: false,
            xauthority: None,
            root: vec![("lib".into(), Some("usr/lib".into())), ("bin".into(), None)],
            devices: vec!["/dev/nvidia0".into()],
        }
    }

    fn args(network: bool) -> Vec<String> {
        let mods = [PathBuf::from("/home/ann/.slop/mods/m/files")];
        let s = Sandbox {
            files: Path::new("/home/ann/.slop/apps/game/files"),
            mods: &mods,
            saves: Path::new("/home/ann/.slop/saves/game"),
            cache: Path::new("/home/ann/.slop/cache/game"),
            network,
        };
        arguments(&s, &host())
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect()
    }

    fn pos(args: &[String], seq: &[&str]) -> usize {
        args.windows(seq.len())
            .position(|w| w.iter().zip(seq).all(|(a, b)| a == b))
            .unwrap_or_else(|| panic!("{seq:?} not in {args:?}"))
    }

    #[test]
    fn only_the_apps_own_folders_are_writable() {
        let a = args(true);
        // Home is empty; the app's folders are mounted on top of it afterwards.
        let home = pos(&a, &["--tmpfs", "/home/ann"]);
        assert!(pos(&a, &["--ro-bind", "/home/ann/.slop/apps/game/files"]) > home);
        assert!(pos(&a, &["--ro-bind", "/home/ann/.slop/mods/m/files"]) > home);
        assert!(pos(&a, &["--bind", "/home/ann/.slop/saves/game"]) > home);
        let writable: Vec<&String> = a
            .windows(2)
            .filter(|w| w[0] == "--bind")
            .map(|w| &w[1])
            .collect();
        assert_eq!(
            writable,
            ["/home/ann/.slop/saves/game", "/home/ann/.slop/cache/game"]
        );
        pos(&a, &["--symlink", "usr/lib", "/lib"]);
        pos(&a, &["--ro-bind", "/bin", "/bin"]);
        pos(&a, &["--dev-bind-try", "/dev/nvidia0"]);
        pos(&a, &["--ro-bind-try", "/run/user/1000/wayland-0"]);
        pos(&a, &["--unsetenv", "DBUS_SESSION_BUS_ADDRESS"]);
        pos(&a, &["--chdir", "/home/ann/.slop/apps/game/files"]);
        assert!(a.contains(&"--share-net".to_string()));
        assert!(!args(false).contains(&"--share-net".to_string()));
    }

    #[test]
    fn settings_per_app_win_over_run_everything() {
        let mut s = SandboxSettings::default();
        assert_eq!(s.for_app("game"), None);
        s.all = true;
        assert!(s.for_app("game").is_some());
        s.apps.insert("game".into(), AppSandbox::default());
        assert_eq!(s.for_app("game"), None, "the player said no for this one");
        s.apps.insert(
            "game".into(),
            AppSandbox {
                on: true,
                block_network: true,
            },
        );
        assert!(s.for_app("game").unwrap().block_network);
    }
}
