//! Sandbox mode (DESIGN §6, SECURITY §6.2): an app started in its own box, where it can read
//! its installed files and mods, write only its save folder, and use the screen, sound, graphics
//! card and controllers. The rest of the computer (documents, browsers, keys, other apps, Slop
//! itself) isn't there. Players choose it per app; Slop suggests it for apps that go online.
//!
//! Linux uses bubblewrap, the sandbox Flatpak is built on. Windows uses an AppContainer, the
//! box Store apps run in (appcontainer.rs).
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
    /// Folders kept read-only: a mods folder in the saves the game reads, which only Slop
    /// fills (DESIGN §9), and Slop Link's answers.
    pub read_only: &'a [PathBuf],
    /// More writable folders: Slop Link's requests.
    pub writable: &'a [PathBuf],
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
    #[cfg(windows)]
    {
        // Making (or finding) a container is the check: it fails where AppContainers can't be
        // used, such as on old Windows or locked-down machines.
        crate::appcontainer::check()
            .map_err(|e| format!("Windows didn't let Slop make a sandbox: {e}"))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Err("sandbox mode isn't available on this system yet".into())
    }
}

/// What the Windows launcher sets up (`--slop-sandbox <spec>`, appcontainer.rs).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Spec {
    /// The app's container: `slop.<address>`.
    pub name: String,
    pub network: bool,
    /// Readable: the app's files and its mods.
    pub read: Vec<PathBuf>,
    /// Writable: its save and cache folders.
    pub write: Vec<PathBuf>,
    /// Inside a writable folder but kept read-only: a mods folder in the saves.
    pub read_only: Vec<PathBuf>,
    pub cwd: PathBuf,
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Spec {
    pub fn new(sandbox: &Sandbox, name: String, program: &Path, args: &[String]) -> Self {
        Spec {
            name,
            network: sandbox.network,
            read: std::iter::once(sandbox.files.to_path_buf())
                .chain(sandbox.mods.iter().cloned())
                .collect(),
            write: [sandbox.saves, sandbox.cache]
                .into_iter()
                .map(Path::to_path_buf)
                .chain(sandbox.writable.iter().cloned())
                .collect(),
            read_only: sandbox.read_only.to_vec(),
            cwd: sandbox.files.to_path_buf(),
            program: program.to_path_buf(),
            args: args.to_vec(),
        }
    }
}

/// An app's container name on Windows: at most 64 characters.
pub fn container_name(slug: &str) -> String {
    format!("slop.{slug}").chars().take(64).collect()
}

/// Quotes one argument for a Windows command line, as the C runtime splits it again.
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

/// The Windows launcher's entry point: the Slop app calls this when started with
/// `--slop-sandbox <spec>`, and exits with what it returns.
#[cfg(windows)]
pub fn launcher_main(spec: &str) -> i32 {
    let spec: Spec = match serde_json::from_str(spec) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("slop sandbox: bad spec: {e}");
            return 125;
        }
    };
    // Temporary files go to the app's cache, which it can write.
    let tmp = spec.write.get(1).map(|c| c.join("tmp"));
    // SAFETY: the launcher is single-threaded at this point.
    unsafe {
        if let Some(tmp) = &tmp
            && std::fs::create_dir_all(tmp).is_ok()
        {
            std::env::set_var("TMP", tmp);
            std::env::set_var("TEMP", tmp);
        }
        std::env::set_var("SLOP_SANDBOX", "1");
    }
    match crate::appcontainer::run(&spec) {
        Ok(code) => code as i32,
        Err(e) => {
            eprintln!("slop sandbox: {e}");
            125
        }
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
pub fn command(
    sandbox: &Sandbox,
    program: &Path,
    args: &[String],
    _name: String,
) -> std::io::Result<Command> {
    let bwrap = bwrap_path().ok_or_else(|| std::io::Error::other("bubblewrap isn't installed"))?;
    let mut cmd = Command::new(bwrap);
    cmd.args(arguments(sandbox, &Host::current()))
        .arg("--")
        .arg(program)
        .args(args)
        .current_dir(sandbox.files);
    Ok(cmd)
}

/// On Windows: the Slop app itself, as the launcher (`--slop-sandbox <spec>`). `name` is the
/// app's container (`container_name`).
#[cfg(windows)]
pub fn command(
    sandbox: &Sandbox,
    program: &Path,
    args: &[String],
    name: String,
) -> std::io::Result<Command> {
    let launcher = match std::env::var_os("SLOP_SANDBOX_LAUNCHER") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe()?,
    };
    let spec = serde_json::to_string(&Spec::new(sandbox, name, program, args))
        .map_err(std::io::Error::other)?;
    let mut cmd = Command::new(launcher);
    cmd.arg("--slop-sandbox")
        .arg(spec)
        .current_dir(sandbox.files);
    Ok(cmd)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn command(_: &Sandbox, _: &Path, _: &[String], _: String) -> std::io::Result<Command> {
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
    for w in s.writable {
        args.extend(bind("--bind", w));
    }
    for r in s.read_only {
        args.extend(bind("--ro-bind", r));
    }
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
        let read_only = [PathBuf::from("/home/ann/.slop/saves/game/mods")];
        let s = Sandbox {
            files: Path::new("/home/ann/.slop/apps/game/files"),
            mods: &mods,
            saves: Path::new("/home/ann/.slop/saves/game"),
            cache: Path::new("/home/ann/.slop/cache/game"),
            read_only: &read_only,
            writable: &[],
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
    fn windows_arguments_survive_quoting() {
        for (arg, quoted) in [
            ("plain", "plain"),
            ("", "\"\""),
            ("two words", "\"two words\""),
            ("C:\\Games\\My Game\\", "\"C:\\Games\\My Game\\\\\""),
            ("say \"hi\"", "\"say \\\"hi\\\"\""),
            ("a\\\\\"b", "\"a\\\\\\\\\\\"b\""),
        ] {
            assert_eq!(quote_windows_arg(arg), quoted, "{arg}");
        }
        assert_eq!(container_name(&"x".repeat(80)).len(), 64);
    }

    #[test]
    fn the_windows_launcher_gets_the_same_folders() {
        let mods = [PathBuf::from("C:/s/mods/m/files")];
        let read_only = [PathBuf::from("C:/s/saves/game/mods")];
        let s = Sandbox {
            files: Path::new("C:/s/apps/game/files"),
            mods: &mods,
            saves: Path::new("C:/s/saves/game"),
            cache: Path::new("C:/s/cache/game"),
            read_only: &read_only,
            writable: &[],
            network: false,
        };
        let spec = Spec::new(
            &s,
            container_name("game"),
            Path::new("C:/s/apps/game/files/game.exe"),
            &["-x".into()],
        );
        assert_eq!(spec.name, "slop.game");
        assert_eq!(
            spec.read,
            [PathBuf::from("C:/s/apps/game/files"), mods[0].clone()]
        );
        assert_eq!(
            spec.write,
            [
                PathBuf::from("C:/s/saves/game"),
                PathBuf::from("C:/s/cache/game")
            ]
        );
        assert_eq!(spec.read_only, read_only);
        assert!(!spec.network);
        let back: Spec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn only_the_apps_own_folders_are_writable() {
        let a = args(true);
        // Home is empty; the app's folders are mounted on top of it afterwards.
        let home = pos(&a, &["--tmpfs", "/home/ann"]);
        assert!(pos(&a, &["--ro-bind", "/home/ann/.slop/apps/game/files"]) > home);
        assert!(pos(&a, &["--ro-bind", "/home/ann/.slop/mods/m/files"]) > home);
        assert!(pos(&a, &["--bind", "/home/ann/.slop/saves/game"]) > home);
        assert!(
            pos(&a, &["--ro-bind", "/home/ann/.slop/saves/game/mods"])
                > pos(&a, &["--bind", "/home/ann/.slop/saves/game"]),
            "a mods folder in the saves stays read-only"
        );
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
