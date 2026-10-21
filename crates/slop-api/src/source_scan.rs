//! Slop's own look at an app's source, at the commit a release was built from (DESIGN §10's
//! "Licence" and "Monetization rules" rows, LEGAL-GUIDANCE IP-2). Slop downloads the
//! repository's archive at that commit once, reads it in memory, and deletes it:
//!
//! - **Licence:** which licence the repository's own licence files are, against the store page's.
//!   Missing or contradicting licence files, source files whose SPDX headers name a different
//!   licence, and (for public apps) licences that aren't OSI-approved become notes.
//! - **Monetization:** apps on Slop can't sell anything (LEGAL §8). Payment libraries, in-app
//!   purchase APIs, checkout links, loot boxes and premium currencies in the code become notes.
//! - **Code from outside Slop:** apps may only run code from releases and mods Slop checked
//!   (Content Policy, "Code from outside Slop"). Code that loads programs, libraries or scripts at
//!   run time, self-updaters, and data formats that can rebuild arbitrary objects (the usual way a
//!   hostile game server takes over a player's computer) become notes.
//! - **Online play:** networking code (engines' multiplayer, sockets, HTTP and WebSocket
//!   clients, online-service SDKs). An app whose store page doesn't say it goes online gets a
//!   note, since players are told about servers Slop can't check. Files with the Slop Pulse
//!   snippet (which only talks to Slop) don't count.
//! - **Packages:** the dependencies named in lockfiles (Cargo.lock, package-lock.json, yarn.lock,
//!   go.mod, poetry.lock, Pipfile.lock, requirements.txt, NuGet's packages.lock.json), kept for
//!   the daily advisory watch (dep_watch.rs).
//!
//! Everything here is a note for the reviewer and the maker: patterns in text have false
//! positives, so nothing is held or rejected by itself.
//!
//! Slop reads repositories on GitHub, GitLab and Codeberg (and other GitLab or Gitea servers
//! listed in `SLOP_SOURCE_HOSTS`, or whose maker saved a read-only token), through each host's
//! archive API. Private ones are read with the maker's token (repo_tokens.rs). Anything else is
//! recorded as not checked, for a reviewer to look at by hand.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    sync::LazyLock,
};

use anyhow::{Context, bail};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use url::Url;

use crate::{AppState, repo_tokens::Kind};

/// Largest repository archive Slop downloads (hosts' archives leave out Git LFS files).
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
/// Text files bigger than this aren't read.
const MAX_TEXT_FILE: u64 = 2 * 1024 * 1024;
/// Stop reading after this much text, so one huge repository can't take the server's time.
const MAX_TEXT_TOTAL: u64 = 256 * 1024 * 1024;
const MAX_PACKAGES: usize = 5000;
const EXAMPLES: usize = 3;
const EVERY: std::time::Duration = std::time::Duration::from_secs(120);
const PER_ROUND: i64 = 3;

/// A dependency, in OSV's ecosystem names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Package {
    pub ecosystem: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub status: &'static str,
    /// Files in the archive, and whether reading stopped early.
    pub files: u64,
    pub truncated: bool,
    pub licence: Licence,
    /// Lockfiles the package list came from.
    pub lockfiles: Vec<String>,
    pub packages: usize,
    pub monetization: Vec<Hit>,
    /// Loading code at run time, self-updaters, and unsafe data formats.
    pub code_loading: Vec<Hit>,
    /// Networking code: the app goes online.
    pub network: Vec<Hit>,
    /// Saves, settings or logs written to fixed places in the user's folders instead of
    /// SLOP_SAVE_DIR, so they won't follow the player into sandbox mode. A note only; left out
    /// when the code reads SLOP_SAVE_DIR.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saves: Vec<Hit>,
    /// Where local saves go, as far as the source shows: `slop` (it reads SLOP_SAVE_DIR),
    /// `engine` (an engine folder the Slop app points into the save folder, like Godot's
    /// `user://`), `elsewhere` (fixed per-user folders: see `saves`) or `none` (no local saves
    /// found). Every checked release says which, so makers and reviewers see it's there.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub save_folder: &'static str,
    /// The store page says it works in sandbox mode, but the code saves elsewhere (set when
    /// the release is checked). Players aren't told it works in sandbox mode while it's so.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sandbox_mismatch: bool,
    /// Code in the repository, by language, plus built code (`.dll`, `.so`, `.pck`…). Shown for
    /// mods: a mod with code gets a full read (DESIGN §9).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub code_files: Vec<Hit>,
    /// A mod with code (set when the release is checked).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mod_code: bool,
    /// A mod with networking code: flagged, and sandbox mode is suggested for its game.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mod_online: bool,
    /// Pay-to-win designs a person should look at (premium currencies, energy skips, VIP tiers,
    /// battle passes, bought boosts). Notes only: nothing on Slop can be sold anyway.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub design: Vec<Hit>,
    /// Slop Pulse in the code: `none`, `start_only` (never sends heartbeats) or `heartbeat`.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub pulse: &'static str,
    /// A community build of a game that counts players, without Pulse (set when checked).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pulse_missing: bool,
    /// It goes online, but its store page doesn't say so (set when the release is checked).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub online_undeclared: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Licence {
    pub declared: String,
    pub files: Vec<LicenceFile>,
    /// SPDX-License-Identifier headers naming something other than the declared licence.
    pub headers: Vec<Hit>,
    /// Licence texts and vendor notices inside files (GPL, Apache, MPL, Unity Asset Store, Epic
    /// Games): code that came from elsewhere, under its own terms.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<Hit>,
    /// Closed-source libraries the code builds against (NVIDIA's Cg runtime): an open-source app
    /// can't depend on them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub closed: Vec<Hit>,
    pub problems: Vec<String>,
    /// A short key per problem (`no_file`, `mismatch`, `header:GPL-3.0`…), so an update's check
    /// can be compared with the last approved release's.
    pub kinds: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LicenceFile {
    pub path: String,
    /// The licence it is, when Slop recognises the text.
    pub spdx: Option<&'static str>,
}

/// A pattern found: what it is, how often, and where (`path:line`).
#[derive(Debug, Serialize)]
pub struct Hit {
    pub rule: String,
    pub label: String,
    pub count: u32,
    pub examples: Vec<String>,
}

/// OSI-approved licences in common use (opensource.org/licenses), by SPDX ID without
/// `-only`/`-or-later`.
const OSI: &[&str] = &[
    "0BSD",
    "AFL-3.0",
    "AGPL-3.0",
    "Apache-2.0",
    "APSL-2.0",
    "Artistic-2.0",
    "BSD-1-Clause",
    "BSD-2-Clause",
    "BSD-2-Clause-Patent",
    "BSD-3-Clause",
    "BSL-1.0",
    "CDDL-1.0",
    "CECILL-2.1",
    "ECL-2.0",
    "EFL-2.0",
    "EPL-1.0",
    "EPL-2.0",
    "EUPL-1.2",
    "GPL-2.0",
    "GPL-3.0",
    "ISC",
    "LGPL-2.1",
    "LGPL-3.0",
    "LPPL-1.3c",
    "MIT",
    "MIT-0",
    "MPL-2.0",
    "MS-PL",
    "MS-RL",
    "MulanPSL-2.0",
    "NCSA",
    "OFL-1.1",
    "OSL-3.0",
    "PostgreSQL",
    "UPL-1.0",
    "Unlicense",
    "Zlib",
    "ZPL-2.1",
    "BlueOak-1.0.0",
    "CC0-1.0",
    "WTFPL",
];

/// Folders whose contents aren't the maker's source (installed packages, build output, engine
/// caches).
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "Library",
    "Temp",
    "obj",
    ".godot",
    ".import",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
];

const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "cs", "gd", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "lua", "c", "cc", "cpp",
    "h", "hpp", "java", "kt", "swift", "html", "htm", "vue", "svelte", "dart", "rb", "php",
];

/// Languages for `code_files`, by extension.
fn language(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "gd" => "GDScript",
        "cs" => "C#",
        "lua" => "Lua",
        "py" => "Python",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" => "TypeScript",
        "rs" => "Rust",
        "go" => "Go",
        "c" | "cc" | "cpp" | "h" | "hpp" => "C or C++",
        "java" | "kt" => "Java or Kotlin",
        "swift" => "Swift",
        "dart" => "Dart",
        "rb" => "Ruby",
        "php" => "PHP",
        "dll" | "so" | "dylib" | "exe" | "pck" | "jar" => "Built code (.dll, .so, .pck…)",
        _ => return None,
    })
}

/// An SPDX ID reduced to its family, so GPL-3.0-only, GPL-3.0-or-later and GPL-3.0+ compare equal.
fn family(id: &str) -> String {
    let id = id.trim();
    let id = id
        .strip_suffix("-only")
        .or_else(|| id.strip_suffix("-or-later"))
        .or_else(|| id.strip_suffix('+'))
        .unwrap_or(id);
    id.to_ascii_lowercase()
}

/// The licence IDs in an SPDX expression (`MIT OR Apache-2.0`, `GPL-3.0-only WITH x`).
pub fn expression_ids(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for word in expr.split(|c: char| c.is_whitespace() || c == '(' || c == ')') {
        if word.is_empty() {
            continue;
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        match word.to_ascii_uppercase().as_str() {
            "AND" | "OR" => {}
            "WITH" => skip_next = true,
            _ => out.push(word.to_string()),
        }
    }
    out
}

/// Recognises a licence file's text. Covers the licences most apps use; anything else is left
/// for the reviewer.
pub fn identify_licence(text: &str) -> Option<&'static str> {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let has = |s: &str| t.to_ascii_lowercase().contains(&s.to_ascii_lowercase());
    if has("GNU AFFERO GENERAL PUBLIC LICENSE") {
        return Some("AGPL-3.0");
    }
    if has("GNU LESSER GENERAL PUBLIC LICENSE") {
        return Some(if has("Version 2.1") {
            "LGPL-2.1"
        } else {
            "LGPL-3.0"
        });
    }
    if has("GNU GENERAL PUBLIC LICENSE") {
        return Some(if has("Version 2,") || has("Version 2 ") {
            "GPL-2.0"
        } else {
            "GPL-3.0"
        });
    }
    if has("Mozilla Public License") && (has("Version 2.0") || has("version 2.0")) {
        return Some("MPL-2.0");
    }
    if has("Apache License") && has("Version 2.0") {
        return Some("Apache-2.0");
    }
    if has("Eclipse Public License") && has("2.0") {
        return Some("EPL-2.0");
    }
    if has("Boost Software License") {
        return Some("BSL-1.0");
    }
    if has("This is free and unencumbered software released into the public domain") {
        return Some("Unlicense");
    }
    if has("CC0 1.0") || has("Creative Commons Legal Code CC0") {
        return Some("CC0-1.0");
    }
    if has("Permission is hereby granted, free of charge, to any person obtaining a copy") {
        return Some(
            if has("The above copyright notice and this permission notice shall be included") {
                "MIT"
            } else {
                "MIT-0"
            },
        );
    }
    if has("Permission to use, copy, modify, and/or distribute this software for any purpose") {
        return Some(
            if has("with or without fee is hereby granted") && has("copyright notice") {
                "ISC"
            } else {
                "0BSD"
            },
        );
    }
    if has("Redistribution and use in source and binary forms") {
        return Some(
            if has("Neither the name") || has("may be used to endorse") {
                "BSD-3-Clause"
            } else {
                "BSD-2-Clause"
            },
        );
    }
    if has("This software is provided 'as-is'")
        && has("Altered source versions must be plainly marked")
    {
        return Some("Zlib");
    }
    None
}

fn is_licence_file(path: &str) -> bool {
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (Some(d), n),
        None => (None, path),
    };
    let upper = name.to_ascii_uppercase();
    match dir {
        None => ["LICENSE", "LICENCE", "COPYING", "UNLICENSE"]
            .iter()
            .any(|p| upper.starts_with(p)),
        // REUSE keeps one file per licence in LICENSES/.
        Some("LICENSES") => true,
        _ => false,
    }
}

struct Rule {
    rule: &'static str,
    label: &'static str,
    re: Regex,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let r = |rule, label, re: &str| Rule {
        rule,
        label,
        re: Regex::new(re).unwrap(),
    };
    vec![
        r(
            "steam_microtransactions",
            "Steam in-game purchases (microtransactions)",
            r"ISteamMicroTxn|MicroTxnAuthorizationResponse",
        ),
        r(
            "unity_iap",
            "Unity In-App Purchasing",
            r"UnityEngine\.Purchasing|com\.unity\.purchasing",
        ),
        r(
            "play_billing",
            "Google Play Billing",
            r"com\.android\.billingclient|launchBillingFlow|GodotGooglePlayBilling",
        ),
        r(
            "apple_iap",
            "Apple in-app purchases (StoreKit)",
            r"SKPaymentQueue|SKProductsRequest|\bimport StoreKit\b|get_singleton\(\s*.InAppStore",
        ),
        r(
            "checkout_link",
            "Checkout or payment links (Stripe, PayPal, Gumroad)",
            r"(?i)checkout\.stripe\.com|buy\.stripe\.com|js\.stripe\.com|api\.stripe\.com|paypal\.com/(cgi-bin|checkout|sdk)|paypalobjects\.com|gumroad\.com/l/",
        ),
        r(
            "loot_box",
            "Loot boxes or gacha",
            r"(?i)\bloot[ _-]?(box|boxes|crate|crates)\b|\bgacha\b|\blootbox",
        ),
        r(
            "premium_currency",
            "Premium currency or currency packs",
            r"(?i)\b(premium|paid|hard)[ _-]?currenc(y|ies)\b|\b(gem|coin|diamond|crystal)[ _-]?(pack|bundle)s?\b",
        ),
    ]
});

/// Code from outside Slop: loading code at run time, self-updaters, and data formats that can
/// rebuild arbitrary objects. (rule, label, file extensions it applies to, pattern)
static CODE_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(|| {
    let r = |rule, label, exts, re: &str| {
        (
            Rule {
                rule,
                label,
                re: Regex::new(re).unwrap(),
            },
            exts,
        )
    };
    vec![
        r(
            "load_assembly",
            "Loads .NET assemblies or Java classes at run time",
            &["cs", "java", "kt"][..],
            r"\bAssembly\.(Load|LoadFrom|LoadFile|UnsafeLoadFrom)\s*\(|\.LoadFromStream\s*\(|\bAppDomain\.\w*\.Load\s*\(|\bURLClassLoader\b|\bDexClassLoader\b|\bdefineClass\s*\(",
        ),
        r(
            "load_native",
            "Loads native libraries at run time",
            &[
                "cs", "c", "cc", "cpp", "h", "hpp", "rs", "py", "go", "gd", "java", "kt",
            ][..],
            r"\bdlopen\s*\(|\bLoadLibrary(A|W|Ex|ExA|ExW)?\s*\(|\bNativeLibrary\.(Load|TryLoad)\s*\(|libloading::|ctypes\.(CDLL|WinDLL|cdll|windll)|\bplugin\.Open\s*\(|GDExtensionManager\.load_extension|System\.loadLibrary\s*\(|System\.load\s*\(",
        ),
        r(
            "load_script",
            "Runs code from text at run time (eval, exec, loadstring)",
            &[
                "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "lua", "gd", "html", "htm", "vue",
                "svelte", "rb", "php", "java", "kt",
            ][..],
            r#"\bloadstring\s*\(|(^|[^.\w])eval\s*\(|(^|[^.\w])exec\s*\(|\bnew\s+Function\s*\(|\bcompile\s*\([^)]*['"]exec['"]|\.source_code\s*=|\bExpression\.new\s*\(|vm\.(runInNewContext|runInThisContext|Script)\b|\bevaluateString\s*\(|\bScriptEngineManager\b|\bcompileString\s*\("#,
        ),
        r(
            "load_resource_pack",
            "Loads packs or scenes from outside the build (Godot resource packs, Unity asset bundles)",
            &["gd", "cs"][..],
            r"\bload_resource_pack\s*\(|AssetBundle\.LoadFrom(File|Memory|Stream)(Async)?\s*\(|UnityWebRequestAssetBundle|Addressables\.LoadContentCatalogAsync",
        ),
        r(
            "self_update",
            "Updates itself outside Slop",
            &["rs", "go", "cs", "js", "mjs", "cjs", "ts", "py", "gd"][..],
            r"\belectron-updater\b|\bautoUpdater\.|\bSquirrel\.|\bVelopack\b|\bNetSparkle\b|\bself_update::|\bgo-selfupdate\b|\bpyupdater\b|\btufup\b",
        ),
        r(
            "unsafe_format",
            "A data format that can rebuild any object (unsafe for data from a server or a file)",
            &[
                "cs", "py", "gd", "java", "kt", "js", "mjs", "cjs", "ts", "rb", "php",
            ][..],
            r"\bBinaryFormatter\b|\bNetDataContractSerializer\b|\bSoapFormatter\b|\bLosFormatter\b|\bObjectStateFormatter\b|TypeNameHandling\.(All|Auto|Objects|Arrays)|\bpickle\.loads?\s*\(|\bcPickle\b|\bdill\.loads?\s*\(|\bmarshal\.loads?\s*\(|\byaml\.(unsafe_)?load\s*\(|\bjsonpickle\.decode\s*\(|bytes_to_var_with_objects|str_to_var\s*\(|\bget_var\s*\(\s*true|\bbytes2var\s*\([^)]*,\s*true|\bObjectInputStream\b|node-serialize|\bunserialize\s*\(|Marshal\.load\s*\(|YAML\.load\s*\(",
        ),
    ]
});

/// Networking code, by engine and language. (rule, label, file extensions, pattern)
static NET_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(|| {
    let r = |rule, label, exts, re: &str| {
        (
            Rule {
                rule,
                label,
                re: Regex::new(re).unwrap(),
            },
            exts,
        )
    };
    vec![
        r(
            "multiplayer",
            "Multiplayer networking (engine or library)",
            &[
                "gd", "cs", "rs", "c", "cc", "cpp", "h", "hpp", "lua", "java", "kt", "js", "ts",
                "py", "go",
            ][..],
            r"\b(ENetMultiplayerPeer|WebSocketMultiplayerPeer|WebRTCMultiplayerPeer|MultiplayerAPI|SceneMultiplayer|NetworkedMultiplayerENet)\b|@rpc\b|\bUnity\.Netcode\b|\busing Mirror\b|\bPhoton(Network|\.Pun|\.Realtime)\b|\bFishNet\b|\bLiteNetLib\b|\bRiptide\b|\bSteamNetworking\w*|\benet_host_create\b|\bSDLNet_|\bbevy_replicon\b|\brenet::|\bnaia_\w+|\bmatchbox_socket\b|\bcolyseus\b|\bkryonet\b|\bgame-networking-sockets\b",
        ),
        r(
            "sockets",
            "Network sockets (TCP, UDP)",
            &[
                "gd", "cs", "rs", "c", "cc", "cpp", "h", "hpp", "lua", "java", "kt", "py", "go",
                "js", "ts",
            ][..],
            r#"\b(StreamPeerTCP|PacketPeerUDP|TCPServer|UDPServer)\b|\b(TcpClient|UdpClient|TcpListener)\b|\bnew Socket\s*\(|\bstd::net::(TcpStream|UdpSocket|TcpListener)|\btokio::net::|\bsocket\.socket\s*\(|\bnet\.(Dial|Listen)\w*\s*\(|\bWSAStartup\b|\bsocket\s*\(\s*AF_INET|\bjava\.net\.(Socket|ServerSocket|DatagramSocket)\b|\bdgram\.createSocket\b|\bnet\.createServer\b|require\s*\(?\s*['"]socket['"]"#,
        ),
        r(
            "http",
            "HTTP or WebSocket clients",
            &[
                "gd", "cs", "rs", "c", "cc", "cpp", "h", "hpp", "lua", "java", "kt", "py", "go",
                "js", "mjs", "cjs", "ts", "tsx", "jsx", "html", "htm", "vue", "svelte", "dart",
                "rb", "php",
            ][..],
            r#"\b(HTTPRequest|HTTPClient|WebSocketPeer)\.new\s*\(|\bUnityWebRequest\b|\bnew HttpClient\s*\(|\bClientWebSocket\b|\breqwest::|\bureq::|\btungstenite\b|\brequests\.(get|post|put|delete|Session)\b|\bhttpx\.|\baiohttp\.|\burllib\.request\b|\bwebsockets\.connect\b|\bhttp\.(Get|Post|NewRequest)\s*\(|(^|[^.\w])fetch\s*\(\s*['"`]https?://|\bnew WebSocket\s*\(|\bXMLHttpRequest\b|\baxios\.|\bio\s*\(\s*['"`]https?://|\bcurl_easy_\w+|\bjava\.net\.http\b|\bHttpURLConnection\b|\bokhttp3\b|\bsocket\.http\b"#,
        ),
        r(
            "online_service",
            "Online services (cloud databases, backends, AI APIs)",
            &[
                "gd", "cs", "rs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "go", "java", "kt",
                "dart", "html", "vue", "svelte",
            ][..],
            r"(?i)\bfirebase\b|\bsupabase\b|\bplayfab\b|\bnakama\b|\bappwrite\b|\bpocketbase\b|\bmongodb(\+srv)?://|\bpostgres(ql)?://|\bmysql://|\bredis://|\bapi\.openai\.com\b|\bapi\.anthropic\.com\b|\bgenerativelanguage\.googleapis\.com\b|\bLootLocker\b|\bGameSparks\b|\bbraincloud\b",
        ),
    ]
});

/// Saves written to fixed places in the user's folders rather than SLOP_SAVE_DIR (sandbox mode,
/// DESIGN §10). Godot's `user://` isn't listed: the Slop app points it at the save folder.
/// (rule, label, file extensions, pattern)
static SAVE_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(|| {
    let r = |rule, label, exts, re: &str| {
        (
            Rule {
                rule,
                label,
                re: Regex::new(re).unwrap(),
            },
            exts,
        )
    };
    vec![
        r(
            "dotnet_folders",
            "Documents or app-data folders (.NET)",
            &["cs", "fs", "vb"][..],
            r"\bEnvironment\.SpecialFolder\.(MyDocuments|Personal|ApplicationData|LocalApplicationData|UserProfile)\b",
        ),
        r(
            "unity_data_path",
            "Unity's persistentDataPath",
            &["cs", "js"][..],
            r"\bApplication\.persistentDataPath\b",
        ),
        r(
            "home_env",
            "The home or app-data folder, from the environment",
            &[
                "gd", "cs", "rs", "c", "cc", "cpp", "h", "hpp", "lua", "java", "kt", "py", "go",
                "js", "mjs", "cjs", "ts", "rb", "php", "dart",
            ][..],
            r#"\b(getenv|env::var(_os)?|environ(\.get)?|System\.getenv|GetEnvironmentVariable|os\.Getenv|get_environment|ENV\.fetch|ENV)\s*[(\[]\s*['"](HOME|APPDATA|USERPROFILE|LOCALAPPDATA|XDG_DATA_HOME|XDG_CONFIG_HOME)['"]|\bprocess\.env\.(HOME|APPDATA|USERPROFILE|LOCALAPPDATA)\b"#,
        ),
        r(
            "home_python",
            "The home folder (Python)",
            &["py"][..],
            r#"\bos\.path\.expanduser\s*\(\s*['"]~|\bPath\.home\s*\(\s*\)"#,
        ),
        r(
            "rust_dirs",
            "The home, data or config folder (Rust)",
            &["rs"][..],
            r"\b(dirs|dirs_next)::(home_dir|data_dir|data_local_dir|config_dir|config_local_dir|document_dir)\b|\b(ProjectDirs|UserDirs|BaseDirs)::(from|new)\b",
        ),
        r(
            "electron_paths",
            "Electron's userData, documents or home folder",
            &["js", "mjs", "cjs", "ts"][..],
            r#"\bapp\.getPath\s*\(\s*['"](userData|documents|home|appData)['"]"#,
        ),
    ]
});

/// Licence texts and vendor notices in files: (rule, label, pattern, declared licence families
/// that make it fine; empty: never fine to publish).
static NOTICE_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(|| {
    let r = |rule, label, re: &str, ok| {
        (
            Rule {
                rule,
                label,
                re: Regex::new(re).unwrap(),
            },
            ok,
        )
    };
    vec![
        r(
            "agpl",
            "the GNU Affero GPL",
            r"GNU Affero General Public License",
            &["AGPL"][..],
        ),
        r(
            "lgpl",
            "the GNU Lesser GPL",
            r"GNU (Lesser|Library) General Public License",
            &["LGPL", "GPL", "AGPL"][..],
        ),
        r(
            "gpl",
            "the GNU GPL",
            r"(this program|you can redistribute it).{0,80}GNU General Public License|GNU General Public License as published by",
            &["GPL", "AGPL"][..],
        ),
        r(
            "apache",
            "the Apache License 2.0",
            r"Licensed under the Apache License, Version 2\.0",
            &["Apache"][..],
        ),
        r(
            "mpl",
            "the Mozilla Public License 2.0",
            r"Mozilla Public License,? v(ersion)? ?2\.0",
            &["MPL"][..],
        ),
        r(
            "unity_asset_store",
            "the Unity Asset Store EULA (Asset Store content can't be published in an open-source repository)",
            r"(?i)asset store (terms of service|eula)|unity-asset-store-eula|standard unity asset store",
            &[][..],
        ),
        r(
            "epic_games",
            "Epic Games' copyright (Unreal Engine code can't be published under an open licence; files you made from Unreal's templates are fine)",
            r"Copyright Epic Games, Inc\. All Rights Reserved",
            &[][..],
        ),
    ]
});

/// Closed-source libraries, found where code includes or links them (not shader source: Unity's
/// `CGPROGRAM` blocks are compiled by Unity, without NVIDIA's runtime).
static CLOSED_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![Rule {
        rule: "nvidia_cg",
        label: "NVIDIA's Cg Toolkit, a closed-source shader library NVIDIA stopped updating in 2012",
        re: Regex::new(
            r#"#\s*include\s*[<"]Cg/cg(GL|D3D\d+)?\.h[>"]|(^|[\s"'])-lCg(GL)?\b|\bfind_package\s*\(\s*Cg\b|\blibCg(GL)?\.so\b|\b[Cc]g(GL)?\.dll\b|link\s*\(\s*name\s*=\s*"Cg(GL)?""#,
        )
        .unwrap(),
    }]
});

/// Pay-to-win designs: notes for a person (DESIGN §8, Content Policy "pay-to-win").
static DESIGN_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let r = |rule, label, re: &str| Rule {
        rule,
        label,
        re: Regex::new(re).unwrap(),
    };
    vec![
        r(
            "premium_currency",
            "A premium currency",
            r"(?i)\b(premium|hard)[_ ]?(currency|gems?|coins?)\b",
        ),
        r(
            "energy_skip",
            "Energy refills or skipping timers",
            r"(?i)\b(energy|stamina)[_ ]?(refill|recharge)\b|\bskip[_ ]?(timer|wait|cooldown)\b",
        ),
        r(
            "vip_tiers",
            "VIP tiers",
            r"(?i)\bvip[_ ]?(level|tier|rank|status)\b",
        ),
        r(
            "battle_pass",
            "A battle or season pass",
            r"(?i)\b(battle|season|premium)[_ ]?pass\b",
        ),
        r(
            "bought_boosts",
            "Boosts or stats for currency",
            r"(?i)\b(buy|purchase)[_ ]?(boost|power[_ ]?up|xp[_ ]?boost|stats?|upgrades?)\b",
        ),
    ]
});

/// Self-updaters: they replace the app with whatever their server sends, bypassing Slop.
const UPDATER_PACKAGES: &[(&str, &str)] = &[
    ("npm", "electron-updater"),
    ("npm", "update-electron-app"),
    ("crates.io", "self_update"),
    ("crates.io", "self-replace"),
    ("crates.io", "velopack"),
    ("NuGet", "Velopack"),
    ("NuGet", "NetSparkleUpdater*"),
    ("NuGet", "Squirrel.Windows"),
    ("NuGet", "Clowd.Squirrel"),
    ("PyPI", "pyupdater"),
    ("PyPI", "tufup"),
    ("Go", "github.com/creativeprojects/go-selfupdate*"),
    ("Go", "github.com/minio/selfupdate*"),
    ("Go", "github.com/rhysd/go-github-selfupdate*"),
];

/// Payment libraries, by ecosystem and name (a trailing `*` matches a prefix).
const PAYMENT_PACKAGES: &[(&str, &str)] = &[
    ("npm", "stripe"),
    ("npm", "@stripe/*"),
    ("npm", "@paypal/*"),
    ("npm", "paypal-rest-sdk"),
    ("npm", "braintree"),
    ("npm", "react-native-iap"),
    ("npm", "react-native-purchases"),
    ("npm", "cordova-plugin-purchase"),
    ("npm", "@revenuecat/*"),
    ("npm", "electron-in-app-purchase"),
    ("PyPI", "stripe"),
    ("PyPI", "paypalrestsdk"),
    ("PyPI", "braintree"),
    ("crates.io", "async-stripe"),
    ("crates.io", "stripe-rust"),
    ("NuGet", "Stripe.net"),
    ("NuGet", "Braintree"),
    ("NuGet", "PayPalCheckoutSdk"),
    ("Go", "github.com/stripe/stripe-go*"),
    ("Maven", "com.android.billingclient:*"),
    ("Maven", "com.revenuecat.purchases:*"),
    ("Maven", "com.stripe:*"),
    ("Unity", "com.unity.purchasing"),
    ("Godot", "GodotGooglePlayBilling*"),
    ("Godot", "InAppStore"),
];

fn is_payment_package(p: &Package) -> bool {
    in_list(PAYMENT_PACKAGES, p)
}

fn in_list(list: &[(&str, &str)], p: &Package) -> bool {
    list.iter().any(|(eco, name)| {
        *eco == p.ecosystem
            && match name.strip_suffix('*') {
                Some(prefix) => p.name.starts_with(prefix),
                None => p.name.eq_ignore_ascii_case(name),
            }
    })
}

static SPDX_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"SPDX-License-Identifier:\s*([A-Za-z0-9.+\-() ]+?)\s*(\*/|-->|#|$)").unwrap()
});

/// The packages a lockfile names. Unknown or damaged files give none.
pub fn parse_lockfile(name: &str, text: &str) -> Vec<Package> {
    let pkg = |eco: &str, name: &str, version: &str| Package {
        ecosystem: eco.into(),
        name: name.into(),
        version: version
            .trim_start_matches('=')
            .trim_start_matches('v')
            .into(),
    };
    let mut out = Vec::new();
    match name {
        "Cargo.lock" | "poetry.lock" => {
            let eco = if name == "Cargo.lock" {
                "crates.io"
            } else {
                "PyPI"
            };
            let Ok(doc) = text.parse::<toml::Table>() else {
                return out;
            };
            for p in doc
                .get("package")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                let (Some(n), Some(v)) = (
                    p.get("name").and_then(|v| v.as_str()),
                    p.get("version").and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                // Cargo: only crates from a registry (not the workspace's own or git ones).
                if eco == "crates.io"
                    && !p
                        .get("source")
                        .and_then(|s| s.as_str())
                        .is_some_and(|s| s.starts_with("registry+"))
                {
                    continue;
                }
                out.push(pkg(eco, n, v));
            }
        }
        "package-lock.json" => {
            let Ok(doc) = serde_json::from_str::<Value>(text) else {
                return out;
            };
            if let Some(packages) = doc["packages"].as_object() {
                for (path, p) in packages {
                    let Some(n) = path.rsplit_once("node_modules/").map(|(_, n)| n) else {
                        continue;
                    };
                    if p["dev"].as_bool() == Some(true) || p["link"].as_bool() == Some(true) {
                        continue;
                    }
                    if let Some(v) = p["version"].as_str() {
                        out.push(pkg("npm", n, v));
                    }
                }
            } else if let Some(deps) = doc["dependencies"].as_object() {
                fn walk(deps: &serde_json::Map<String, Value>, out: &mut Vec<Package>) {
                    for (n, p) in deps {
                        if p["dev"].as_bool() == Some(true) {
                            continue;
                        }
                        if let Some(v) = p["version"].as_str() {
                            out.push(Package {
                                ecosystem: "npm".into(),
                                name: n.clone(),
                                version: v.into(),
                            });
                        }
                        if let Some(inner) = p["dependencies"].as_object() {
                            walk(inner, out);
                        }
                    }
                }
                walk(deps, &mut out);
            }
        }
        "yarn.lock" => {
            // Yarn 1: `"name@range", "name@range2":` then an indented `version "x"`.
            let mut current: Option<String> = None;
            for line in text.lines() {
                if !line.starts_with(' ') && line.ends_with(':') && !line.starts_with('#') {
                    let first = line
                        .trim_end_matches(':')
                        .split(", ")
                        .next()
                        .unwrap_or("")
                        .trim_matches('"');
                    current = first
                        .rfind('@')
                        .filter(|&i| i > 0)
                        .map(|i| first[..i].to_string());
                } else if let (Some(n), Some(v)) =
                    (current.as_ref(), line.trim().strip_prefix("version "))
                {
                    out.push(pkg("npm", n, v.trim_matches('"')));
                    current = None;
                }
            }
        }
        "go.mod" => {
            let mut in_block = false;
            for line in text.lines() {
                let line = line.split("//").next().unwrap_or("").trim();
                let spec = if in_block {
                    if line == ")" {
                        in_block = false;
                        continue;
                    }
                    line
                } else if line == "require (" {
                    in_block = true;
                    continue;
                } else if let Some(rest) = line.strip_prefix("require ") {
                    rest
                } else {
                    continue;
                };
                let mut parts = spec.split_whitespace();
                if let (Some(n), Some(v)) = (parts.next(), parts.next()) {
                    out.push(pkg("Go", n, v));
                }
            }
        }
        "Pipfile.lock" => {
            let Ok(doc) = serde_json::from_str::<Value>(text) else {
                return out;
            };
            for (n, p) in doc["default"].as_object().into_iter().flatten() {
                if let Some(v) = p["version"].as_str() {
                    out.push(pkg("PyPI", n, v));
                }
            }
        }
        "packages.lock.json" => {
            let Ok(doc) = serde_json::from_str::<Value>(text) else {
                return out;
            };
            for framework in doc["dependencies"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, f)| f)
            {
                for (n, p) in framework.as_object().into_iter().flatten() {
                    if p["type"].as_str() == Some("Project") {
                        continue;
                    }
                    if let Some(v) = p["resolved"].as_str() {
                        out.push(pkg("NuGet", n, v));
                    }
                }
            }
        }
        "pnpm-lock.yaml" => {
            // `packages:` then one key per package, indented two spaces: `/name@1.2.3:` (v6),
            // `name@1.2.3:` or `'@scope/name@1.2.3(peer)':` (v9), `/name/1.2.3:` (v5).
            let mut in_packages = false;
            for line in text.lines() {
                if !line.starts_with(' ') && !line.is_empty() {
                    in_packages = line.trim_end() == "packages:";
                    continue;
                }
                let Some(key) = line.strip_prefix("  ").filter(|k| !k.starts_with(' ')) else {
                    continue;
                };
                if !in_packages || !key.trim_end().ends_with(':') {
                    continue;
                }
                let key = key
                    .trim_end()
                    .trim_end_matches(':')
                    .trim_matches(['\'', '"']);
                let key = key.strip_prefix('/').unwrap_or(key);
                let key = key.split('(').next().unwrap_or(key);
                let split = key
                    .rfind('@')
                    .filter(|&i| i > 0)
                    .or_else(|| key.rfind('/').filter(|&i| i > 0));
                if let Some(i) = split {
                    let version = key[i + 1..].split('_').next().unwrap_or("");
                    if !version.is_empty() {
                        out.push(pkg("npm", &key[..i], version));
                    }
                }
            }
        }
        "gradle.lockfile" => {
            // `group:artifact:version=classpaths`; test-only dependencies don't ship.
            for line in text.lines() {
                let Some((coords, paths)) = line.trim().split_once('=') else {
                    continue;
                };
                if line.starts_with('#') || coords == "empty" {
                    continue;
                }
                if paths.split(',').all(|p| p.trim().starts_with("test")) {
                    continue;
                }
                let mut parts = coords.split(':');
                if let (Some(g), Some(a), Some(v)) = (parts.next(), parts.next(), parts.next()) {
                    out.push(pkg("Maven", &format!("{g}:{a}"), v));
                }
            }
        }
        "packages-lock.json" => {
            // Unity's resolved packages; built-in and embedded ones aren't third-party.
            let Ok(doc) = serde_json::from_str::<Value>(text) else {
                return out;
            };
            for (n, p) in doc["dependencies"].as_object().into_iter().flatten() {
                if matches!(p["source"].as_str(), Some("builtin" | "embedded" | "local")) {
                    continue;
                }
                if let Some(v) = p["version"].as_str() {
                    out.push(pkg("Unity", n, v));
                }
            }
        }
        "plugin.cfg" => {
            // A Godot add-on (addons/<name>/plugin.cfg): its name and version.
            let field = |key: &str| {
                text.lines().find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
                })
            };
            if let (Some(n), Some(v)) = (field("name"), field("version")) {
                out.push(pkg("Godot", &n, &v));
            }
        }
        _ if name.starts_with("requirements") && name.ends_with(".txt") => {
            for line in text.lines() {
                let line = line.split('#').next().unwrap_or("").trim();
                if let Some((n, v)) = line.split_once("==") {
                    let n = n.split('[').next().unwrap_or(n).trim();
                    let v = v.split(';').next().unwrap_or(v).trim();
                    if !n.is_empty() && !v.is_empty() {
                        out.push(pkg("PyPI", n, v));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn is_lockfile(name: &str) -> bool {
    matches!(
        name,
        "Cargo.lock"
            | "package-lock.json"
            | "yarn.lock"
            | "go.mod"
            | "poetry.lock"
            | "Pipfile.lock"
            | "packages.lock.json"
            | "pnpm-lock.yaml"
            | "gradle.lockfile"
            | "packages-lock.json"
    ) || (name.starts_with("requirements") && name.ends_with(".txt"))
}

#[derive(Default)]
struct Tally {
    hits: BTreeMap<String, (String, u32, Vec<String>)>,
}

impl Tally {
    fn add(&mut self, rule: &str, label: &str, at: String) {
        let e = self
            .hits
            .entry(rule.to_string())
            .or_insert_with(|| (label.to_string(), 0, Vec::new()));
        e.1 += 1;
        if e.2.len() < EXAMPLES {
            e.2.push(at);
        }
    }

    fn into_hits(self) -> Vec<Hit> {
        self.hits
            .into_iter()
            .map(|(rule, (label, count, examples))| Hit {
                rule,
                label,
                count,
                examples,
            })
            .collect()
    }
}

/// Joins hits with the same label (`js` and `mjs` are both JavaScript).
fn merge_by_label(hits: Vec<Hit>) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::new();
    for h in hits {
        match out.iter_mut().find(|o| o.label == h.label) {
            Some(o) => {
                o.count += h.count;
                for e in h.examples {
                    if o.examples.len() < EXAMPLES {
                        o.examples.push(e);
                    }
                }
            }
            None => out.push(h),
        }
    }
    out
}

/// Reads a repository archive (`.tar.gz`, one top folder, as GitHub, GitLab and Gitea make them).
pub fn scan_archive(
    archive: impl Read,
    declared: &str,
    public: bool,
) -> anyhow::Result<(Report, Vec<Package>)> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut report = Report {
        status: "checked",
        ..Default::default()
    };
    let declared_families: BTreeSet<String> = expression_ids(declared)
        .iter()
        .map(|id| family(id))
        .collect();
    let mut packages = BTreeSet::new();
    let mut monetization = Tally::default();
    let mut headers = Tally::default();
    let mut notices: Vec<(&'static str, &'static str, &'static [&'static str], String)> =
        Vec::new();
    let mut notice_tally = Tally::default();
    let mut closed = Tally::default();
    let mut design = Tally::default();
    let (mut pulse_uses, mut pulse_beats) = (false, false);
    let mut code_loading = Tally::default();
    let mut network = Tally::default();
    let mut saves = Tally::default();
    let mut code_files = Tally::default();
    // Code that reads SLOP_SAVE_DIR knows about Slop's save folder: no save notes then.
    let mut uses_save_dir = false;
    // Godot's user:// lands in the save folder by itself.
    let mut engine_saves = false;
    let mut text_total: u64 = 0;
    for entry in tar.entries().context("reading the repository archive")? {
        let mut entry = entry.context("reading the repository archive")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let full = entry.path()?.to_string_lossy().into_owned();
        // Archives put everything under one folder, such as GitHub's `<owner>-<repo>-<sha>/`.
        let Some((_, path)) = full.split_once('/') else {
            continue;
        };
        let path = path.to_string();
        report.files += 1;
        if path.split('/').any(|part| SKIP_DIRS.contains(&part)) {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if let Some(lang) = language(&ext) {
            code_files.add(&ext, lang, path.clone());
        }
        let licence_file = is_licence_file(&path);
        let lockfile = is_lockfile(&name)
            || (name == "plugin.cfg" && (path.starts_with("addons/") || path.contains("/addons/")));
        let unity_manifest = path.ends_with("Packages/manifest.json");
        let source = SOURCE_EXTS.contains(&ext.as_str()) || ext == "cfg" && name == "project.godot";
        // Build scripts, for libraries the program links.
        let build_file = matches!(
            name.as_str(),
            "CMakeLists.txt"
                | "Makefile"
                | "makefile"
                | "GNUmakefile"
                | "meson.build"
                | "SConstruct"
        ) || matches!(ext.as_str(), "cmake" | "mk" | "pro" | "vcxproj");
        // Third-party notices shipped inside the project (Asset Store packages, vendored code).
        let lower = name.to_ascii_lowercase();
        let notice_file = !licence_file
            && matches!(ext.as_str(), "txt" | "md" | "")
            && (lower.contains("license") || lower.contains("licence") || lower.contains("eula"));
        if !(licence_file || lockfile || unity_manifest || source || notice_file || build_file) {
            continue;
        }
        let size = entry.header().size().unwrap_or(0);
        if size > MAX_TEXT_FILE {
            continue;
        }
        if text_total + size > MAX_TEXT_TOTAL {
            report.truncated = true;
            break;
        }
        text_total += size;
        let mut bytes = Vec::with_capacity(size as usize);
        entry.read_to_end(&mut bytes)?;
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if licence_file {
            report.licence.files.push(LicenceFile {
                path: path.clone(),
                spdx: identify_licence(&text),
            });
        }
        if lockfile {
            let found = parse_lockfile(&name, &text);
            if !found.is_empty() {
                report.lockfiles.push(path.clone());
                packages.extend(found);
            }
        }
        if source || build_file {
            // Once per file, at its first mention.
            for rule in CLOSED_RULES.iter() {
                if let Some(i) = text
                    .lines()
                    .position(|l| l.len() <= 2000 && rule.re.is_match(l))
                {
                    closed.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                }
            }
        }
        if notice_file || source {
            for (rule, ok) in NOTICE_RULES.iter() {
                if let Some(i) = text.lines().take(60).position(|l| rule.re.is_match(l)) {
                    notices.push((rule.rule, rule.label, ok, format!("{path}:{}", i + 1)));
                }
            }
        }
        if unity_manifest || source {
            // The Slop Pulse snippet only talks to Slop.
            let pulse = text.contains("SLOP_PULSE");
            if text.contains("SLOP_PULSE_TOKEN") {
                pulse_uses = true;
                pulse_beats |= text.contains("heartbeat");
            }
            uses_save_dir |= text.contains("SLOP_SAVE_DIR");
            engine_saves |=
                matches!(ext.as_str(), "gd" | "cs" | "godot" | "cfg") && text.contains("user://");
            for (i, line) in text.lines().enumerate() {
                if line.len() > 2000 {
                    continue;
                }
                for rule in RULES.iter() {
                    if rule.re.is_match(line) {
                        monetization.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                for (rule, exts) in CODE_RULES.iter() {
                    if exts.contains(&ext.as_str()) && rule.re.is_match(line) {
                        code_loading.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                for (rule, exts) in NET_RULES.iter() {
                    if !pulse && exts.contains(&ext.as_str()) && rule.re.is_match(line) {
                        network.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                for (rule, exts) in SAVE_RULES.iter() {
                    if exts.contains(&ext.as_str()) && rule.re.is_match(line) {
                        saves.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                    }
                }
                if source {
                    for rule in DESIGN_RULES.iter() {
                        if rule.re.is_match(line) {
                            design.add(rule.rule, rule.label, format!("{path}:{}", i + 1));
                        }
                    }
                }
                if i < 30
                    && let Some(m) = SPDX_HEADER.captures(line)
                {
                    let expr = m[1].trim();
                    let ids = expression_ids(expr);
                    if !ids.is_empty()
                        && !ids.iter().any(|id| declared_families.contains(&family(id)))
                    {
                        headers.add(expr, expr, format!("{path}:{}", i + 1));
                    }
                }
            }
        }
    }
    let packages: Vec<Package> = packages.into_iter().take(MAX_PACKAGES).collect();
    for p in packages.iter().filter(|p| is_payment_package(p)) {
        monetization.add(
            "payment_library",
            "A payment library",
            format!("{} {} ({})", p.name, p.version, p.ecosystem),
        );
    }
    for p in packages.iter().filter(|p| in_list(UPDATER_PACKAGES, p)) {
        code_loading.add(
            "self_update",
            "Updates itself outside Slop",
            format!("{} {} ({})", p.name, p.version, p.ecosystem),
        );
    }
    report.packages = packages.len();
    report.monetization = monetization.into_hits();
    report.code_loading = code_loading.into_hits();
    report.network = network.into_hits();
    if !uses_save_dir {
        report.saves = saves.into_hits();
    }
    report.code_files = merge_by_label(code_files.into_hits());
    report.design = design.into_hits();
    report.pulse = match (pulse_uses, pulse_beats) {
        (false, _) => "none",
        (true, false) => "start_only",
        (true, true) => "heartbeat",
    };
    // Licence notices the store page's licence doesn't cover.
    for (rule, label, ok, at) in notices {
        let covered = !ok.is_empty()
            && declared_families
                .iter()
                .any(|f| ok.iter().any(|o| f.starts_with(&o.to_ascii_lowercase())));
        if !covered {
            notice_tally.add(rule, label, at);
        }
    }
    report.licence.notices = notice_tally.into_hits();
    report.licence.closed = closed.into_hits();
    report.save_folder = if uses_save_dir {
        "slop"
    } else if !report.saves.is_empty() {
        "elsewhere"
    } else if engine_saves {
        "engine"
    } else {
        "none"
    };
    report.licence.headers = headers.into_hits();
    report.licence.declared = declared.to_string();
    (report.licence.kinds, report.licence.problems) =
        licence_problems(&report.licence, &declared_families, public)
            .into_iter()
            .unzip();
    Ok((report, packages))
}

fn licence_problems(
    l: &Licence,
    declared: &BTreeSet<String>,
    public: bool,
) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    if l.files.is_empty() {
        problems.push((
            "no_file".into(),
            "There's no licence file (LICENSE or COPYING) at the top of the repository, so nobody has permission to use the code. Add the licence's full text as LICENSE.".into(),
        ));
    } else {
        let known: Vec<&str> = l.files.iter().filter_map(|f| f.spdx).collect();
        if known.is_empty() {
            problems.push((
                "unidentified".into(),
                format!(
                    "Slop couldn't tell which licence {} is. A reviewer checks that it's {}.",
                    l.files[0].path, l.declared
                ),
            ));
        } else if !known.iter().any(|id| declared.contains(&family(id))) {
            problems.push((
                "mismatch".into(),
                format!(
                    "The licence file says {}, but the store page says {}. Make them match.",
                    known.join(" and "),
                    l.declared
                ),
            ));
        }
    }
    for h in &l.headers {
        problems.push((
            format!("header:{}", h.rule),
            format!(
                "{} source file{} say{} {} in their headers (e.g. {}), not the store page's {}.",
                h.count,
                if h.count == 1 { "" } else { "s" },
                if h.count == 1 { "s" } else { "" },
                h.label,
                h.examples.join(", "),
                l.declared
            ),
        ));
    }
    for c in &l.closed {
        problems.push((
            format!("closed:{}", c.rule),
            format!(
                "{} file{} build{} against {} (e.g. {}). An open-source app can't depend on a closed-source library: use GLSL or HLSL shaders instead.",
                c.count,
                if c.count == 1 { "" } else { "s" },
                if c.count == 1 { "s" } else { "" },
                c.label,
                c.examples.join(", "),
            ),
        ));
    }
    for n in &l.notices {
        problems.push((
            format!("notice:{}", n.rule),
            format!(
                "{} file{} carr{} {} (e.g. {}), which the store page's {} doesn't cover. Code from elsewhere keeps its own terms: name its licence on the store page too, or take it out.",
                n.count,
                if n.count == 1 { "" } else { "s" },
                if n.count == 1 { "ies" } else { "y" },
                n.label,
                n.examples.join(", "),
                l.declared
            ),
        ));
    }
    if public {
        let not_osi: Vec<String> = expression_ids(&l.declared)
            .into_iter()
            .filter(|id| !OSI.iter().any(|o| family(o) == family(id)))
            .collect();
        if !not_osi.is_empty() {
            problems.push((
                "not_osi".into(),
                format!(
                    "{} isn't on Slop's list of OSI-approved licences, which apps in the store need (opensource.org/licenses).",
                    not_osi.join(", ")
                ),
            ));
        }
    }
    problems
}

/// The repository is bigger than Slop reads: recorded as not checked, not as a failure.
#[derive(Debug, thiserror::Error)]
#[error("the repository is bigger than the {} MiB Slop reads, so a reviewer checks the licence by hand", MAX_ARCHIVE / 1024 / 1024)]
struct TooBig;

/// Where a repository's archive at a commit comes from.
#[derive(Debug, PartialEq, Eq)]
pub struct Source {
    /// The host's name for messages ("GitHub", "GitLab", "Codeberg").
    pub host: String,
    pub url: Url,
    /// Whether it's GitHub's API, asked with Slop's GitHub App or token.
    pub github: bool,
}

/// The archive's address on a GitLab or Gitea host: `https://host/group/sub/repo` at `commit`.
pub fn archive_url(kind: Kind, repo_url: &Url, commit: &str) -> Option<Url> {
    let path = repo_url.path().trim_matches('/').trim_end_matches(".git");
    if !path.contains('/') {
        return None;
    }
    let origin = repo_url.origin().ascii_serialization();
    let url = match kind {
        Kind::Gitlab => format!(
            "{origin}/api/v4/projects/{}/repository/archive.tar.gz?sha={commit}",
            percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC)
        ),
        Kind::Gitea => format!("{origin}/api/v1/repos/{path}/archive/{commit}.tar.gz"),
        // Bitbucket Cloud's download link for a commit.
        Kind::Bitbucket => format!("{origin}/{path}/get/{commit}.tar.gz"),
    };
    Url::parse(&url).ok()
}

/// What software a self-hosted server runs, by asking it: Gitea and Forgejo answer
/// `/api/v1/version`, GitLab its project API for a public project. Never on a private network,
/// and in tests only on this machine (tests never reach the internet).
async fn detect(state: &AppState, repo: &Url) -> Option<Kind> {
    let host = repo.host_str()?;
    if state.config.releases.allow_http_downloads && !matches!(host, "127.0.0.1" | "localhost") {
        return None;
    }
    let origin = repo.origin().ascii_serialization();
    let path = repo.path().trim_matches('/').trim_end_matches(".git");
    let ask = |url: String, key: &'static str| async move {
        let url = Url::parse(&url).ok()?;
        crate::repo_owner::public_host(state, &url).await.ok()?;
        let res = state
            .http
            .get(url)
            .header("user-agent", "slop-api")
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        let body: Value = res.json().await.ok()?;
        body[key].is_string().then_some(())
    };
    if ask(format!("{origin}/api/v1/version"), "version")
        .await
        .is_some()
    {
        return Some(Kind::Gitea);
    }
    let project = percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC);
    if ask(
        format!("{origin}/api/v4/projects/{project}"),
        "path_with_namespace",
    )
    .await
    .is_some()
    {
        return Some(Kind::Gitlab);
    }
    None
}

/// Where to read a release's source, or None for hosts Slop can't read.
async fn source(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
    commit: &str,
) -> anyhow::Result<Option<Source>> {
    // A commit is 40 (or 64) hex digits; anything else never reaches a URL.
    anyhow::ensure!(
        !commit.is_empty() && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "the release's commit isn't a commit ID"
    );
    if let Some(repo) = slop_provenance::github_repo_path(repo_url) {
        return github_source(state, &repo, commit).map(Some);
    }
    let Ok(url) = Url::parse(repo_url) else {
        return Ok(None);
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    // A token the maker saved says what the host is (self-hosted servers); then the list; then
    // asking the server itself.
    let saved = crate::repo_tokens::saved_kind(&state.db, app_id).await?;
    // Development and tests: a stand-in GitHub serves repositories under its own address.
    let base = state.config.releases.github_api_base.as_str();
    let stand_in = state
        .config
        .releases
        .allow_http_downloads
        .then(|| repo_url.strip_prefix(base))
        .flatten()
        .map(|p| p.trim_matches('/').to_string())
        .filter(|p| p.split('/').count() == 2);
    if let (None, Some(repo)) = (saved, stand_in) {
        return github_source(state, &repo, commit).map(Some);
    }
    let kind = match saved {
        Some(kind) => Some(kind),
        None => match state
            .config
            .releases
            .source_hosts
            .iter()
            .find(|(h, _)| *h == host)
            .map(|(_, k)| *k)
        {
            Some(k) => Some(k),
            None => detect(state, &url).await,
        },
    };
    if let Some(archive) = kind.and_then(|k| archive_url(k, &url, commit)) {
        let name = match (host.as_str(), kind) {
            ("codeberg.org", _) => "Codeberg".to_string(),
            ("bitbucket.org", _) => "Bitbucket".to_string(),
            (_, Some(Kind::Gitlab)) => "GitLab".to_string(),
            _ => host.clone(),
        };
        return Ok(Some(Source {
            host: name,
            url: archive,
            github: false,
        }));
    }
    Ok(None)
}

fn github_source(state: &AppState, repo: &str, commit: &str) -> anyhow::Result<Source> {
    Ok(Source {
        host: "GitHub".into(),
        url: state
            .config
            .releases
            .github_api_base
            .join(&format!("repos/{repo}/tarball/{commit}"))?,
        github: true,
    })
}

/// Downloads the repository archive to a temporary file.
async fn download_archive(
    state: &AppState,
    app_id: Uuid,
    source: &Source,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let host = &source.host;
    let request = if source.github {
        crate::releases::github_get(state, source.url.clone()).await
    } else {
        // Self-hosted servers: never one on a private network.
        crate::repo_owner::public_host(state, &source.url).await?;
        let mut req = state
            .fetch_http
            .get(source.url.clone())
            .header("user-agent", "slop-api");
        // Private repositories: the maker's read-only token, only ever sent to its own host.
        if let Some(auth) = crate::repo_tokens::auth_for(state, app_id, source.url.as_str()).await?
        {
            req = req.header("authorization", auth);
        }
        req
    };
    let res = request.send().await.context("downloading the source")?;
    match res.status().as_u16() {
        200 => {}
        401 | 403 | 404 => bail!(
            "{host} can't find that commit (or the repository isn't visible to Slop; for a private repository, save a read-only token)"
        ),
        code => bail!("{host} answered {code} for the source archive"),
    }
    let mut out = tokio::fs::File::create(path).await?;
    let mut size = 0u64;
    let mut res = res;
    while let Some(chunk) = res.chunk().await.context("source download interrupted")? {
        size += chunk.len() as u64;
        if size > MAX_ARCHIVE {
            return Err(TooBig.into());
        }
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    Ok(())
}

/// Looks at one release's source. Returns the report to store and the package list.
pub async fn scan_release(
    state: &AppState,
    app_id: Uuid,
    repo_url: &str,
    commit: &str,
    declared: &str,
    public: bool,
) -> (Value, Vec<Package>) {
    let path = std::env::temp_dir().join(format!("slop-source-{}.tar.gz", Uuid::now_v7()));
    let result = async {
        let Some(source) = source(state, app_id, repo_url, commit).await? else {
            return Ok(None);
        };
        download_archive(state, app_id, &source, &path).await?;
        let (declared, file) = (declared.to_string(), std::fs::File::open(&path)?);
        tokio::task::spawn_blocking(move || {
            scan_archive(std::io::BufReader::new(file), &declared, public)
        })
        .await?
        .map(Some)
    }
    .await;
    let _ = tokio::fs::remove_file(&path).await;
    match result {
        Ok(Some((report, packages))) => {
            (serde_json::to_value(report).unwrap_or_default(), packages)
        }
        Ok(None) => (
            json!({ "status": "not_checked", "detail": "Slop can only read source on GitHub, GitLab, Bitbucket, Codeberg and other Gitea or Forgejo servers so far; a reviewer checks the licence by hand." }),
            vec![],
        ),
        Err(err) if err.downcast_ref::<TooBig>().is_some() => (
            json!({ "status": "not_checked", "detail": err.to_string() }),
            vec![],
        ),
        Err(err) => (
            json!({ "status": "failed", "detail": format!("{err:#}") }),
            vec![],
        ),
    }
}

/// One round: releases waiting for their source check, and failed ones once an hour.
pub async fn run_once(state: &AppState) -> anyhow::Result<usize> {
    let due: Vec<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "select r.id, a.repo_url, r.commit_sha, a.license_spdx, a.visibility = 'public'
         from releases r join apps a on a.id = r.app_id
         where r.status in ('awaiting_review', 'approved')
           and (r.repo_scanned_at is null
                or (r.repo_scan ->> 'status' = 'failed' and r.repo_scanned_at < now() - interval '1 hour'
                    and r.created_at > now() - interval '7 days'))
         order by r.repo_scanned_at nulls first, r.created_at desc
         limit $1",
    )
    .bind(PER_ROUND)
    .fetch_all(&state.db)
    .await?;
    let n = due.len();
    for (id, ..) in due {
        check_release(state, id).await?;
    }
    Ok(n)
}

/// Checks one release's source now and stores the result. Called when a release's files are
/// fetched, before automatic approval, and by the background round for the rest.
pub async fn check_release(state: &AppState, id: Uuid) -> anyhow::Result<()> {
    type Row = (Uuid, String, String, String, bool, bool, bool, bool);
    let (app_id, repo_url, commit, declared, public, online, sandbox_ready, is_mod): Row = sqlx::query_as(
        "select a.id, a.repo_url, r.commit_sha, a.license_spdx, a.visibility = 'public',
                cardinality(a.online) > 0, a.sandbox_ready, a.branch_kind is not distinct from 'addon'
         from releases r join apps a on a.id = r.app_id where r.id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    {
        let (mut report, packages) =
            scan_release(state, app_id, &repo_url, &commit, &declared, public).await;
        // Players are told about apps that go online; one that doesn't say so needs a look.
        if !online && report["network"].as_array().is_some_and(|n| !n.is_empty()) {
            report["online_undeclared"] = json!(true);
        }
        // A mod may carry code (DESIGN §9): flagged for a full read, and one that goes online,
        // declared or not, is flagged and suggests sandbox mode for its game.
        if is_mod && report["status"] == "checked" {
            if report["code_files"]
                .as_array()
                .is_some_and(|c| !c.is_empty())
            {
                report["mod_code"] = json!(true);
            }
            if report["network"].as_array().is_some_and(|n| !n.is_empty()) {
                report["mod_online"] = json!(true);
            }
        }
        // A community build of a game that counts players must count them too (DESIGN §9).
        if report["status"] == "checked"
            && report["pulse"] == "none"
            && crate::branches::needs_pulse(&mut *state.db.acquire().await?, app_id).await?
        {
            report["pulse_missing"] = json!(true);
        }
        // "Works in sandbox mode" means local saves only in SLOP_SAVE_DIR.
        if sandbox_ready && report["save_folder"] == "elsewhere" {
            report["sandbox_mismatch"] = json!(true);
        }
        let mut tx = state.db.begin().await?;
        sqlx::query("update releases set repo_scan = $2, repo_scanned_at = now() where id = $1")
            .bind(id)
            .bind(&report)
            .execute(&mut *tx)
            .await?;
        if !packages.is_empty() {
            sqlx::query("delete from release_packages where release_id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "insert into release_packages (release_id, ecosystem, name, version)
                 select $1, * from unnest($2::text[], $3::text[], $4::text[]) on conflict do nothing",
            )
            .bind(id)
            .bind(packages.iter().map(|p| p.ecosystem.clone()).collect::<Vec<_>>())
            .bind(packages.iter().map(|p| p.name.clone()).collect::<Vec<_>>())
            .bind(packages.iter().map(|p| p.version.clone()).collect::<Vec<_>>())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

/// The concerns in a check (monetization rules, licence problem kinds), or None when the source
/// couldn't be read.
fn concerns(scan: &Value) -> Option<BTreeSet<String>> {
    match scan["status"].as_str() {
        Some("checked") => {}
        Some("not_checked") => return Some(BTreeSet::new()),
        _ => return None,
    }
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(String::from))
            .collect()
    };
    let mut out: BTreeSet<String> = strings(&scan["licence"]["kinds"])
        .into_iter()
        .map(|k| format!("licence:{k}"))
        .collect();
    if scan["online_undeclared"] == json!(true) {
        out.insert("online:undeclared".into());
    }
    if scan["sandbox_mismatch"] == json!(true) {
        out.insert("saves:sandbox_mismatch".into());
    }
    for flag in ["mod_code", "mod_online"] {
        if scan[flag] == json!(true) {
            out.insert(format!("mod:{flag}"));
        }
    }
    for (section, key) in [("monetization", "monetization"), ("code_loading", "code")] {
        for hit in scan[section].as_array().into_iter().flatten() {
            if let Some(rule) = hit["rule"].as_str() {
                out.insert(format!("{key}:{rule}"));
            }
        }
    }
    Some(out)
}

/// Why an update needs a person after all: its source check found something the app's last
/// approved release didn't have (a new monetization pattern or licence problem), or the source
/// couldn't be read. None when there's nothing new.
pub async fn new_concerns(db: &sqlx::PgPool, release_id: Uuid) -> anyhow::Result<Option<String>> {
    let (this, before): (Option<Value>, Option<Value>) = sqlx::query_as(
        "select r.repo_scan,
                (select p.repo_scan from releases p
                 where p.app_id = r.app_id and p.id <> r.id and p.status = 'approved'
                 order by p.reviewed_at desc limit 1)
         from releases r where r.id = $1",
    )
    .bind(release_id)
    .fetch_one(db)
    .await?;
    let Some(now) = concerns(this.as_ref().unwrap_or(&Value::Null)) else {
        return Ok(Some(
            "Slop couldn't read this release's source, so a person checks it".into(),
        ));
    };
    let before = before.as_ref().and_then(concerns).unwrap_or_default();
    let new: Vec<String> = now.difference(&before).cloned().collect();
    if new.is_empty() {
        return Ok(None);
    }
    let kinds: Vec<&str> = [
        ("monetization:", "possible purchases or loot boxes"),
        ("code:", "code loading from outside Slop"),
        ("online:", "online play the store page doesn't mention"),
        ("licence:", "licence problems"),
    ]
    .into_iter()
    .filter(|(prefix, _)| new.iter().any(|k| k.starts_with(prefix)))
    .map(|(_, what)| what)
    .collect();
    let what = match kinds.as_slice() {
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => unreachable!("new is not empty"),
    };
    Ok(Some(format!(
        "the source check found {what} the last approved release didn't have"
    )))
}

pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(EVERY);
    loop {
        tick.tick().await;
        if let Err(err) = run_once(&state).await {
            tracing::error!(error = %err, "source checks");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_come_from_each_hosts_own_address() {
        let url = |s: &str| Url::parse(s).unwrap();
        let c = "a".repeat(40);
        assert_eq!(
            archive_url(
                Kind::Bitbucket,
                &url("https://bitbucket.org/ws/game.git"),
                &c
            )
            .unwrap()
            .as_str(),
            format!("https://bitbucket.org/ws/game/get/{c}.tar.gz")
        );
        assert_eq!(
            archive_url(Kind::Gitea, &url("https://git.example/me/game"), &c)
                .unwrap()
                .as_str(),
            format!("https://git.example/api/v1/repos/me/game/archive/{c}.tar.gz")
        );
        assert!(archive_url(Kind::Gitea, &url("https://git.example/onlyone"), &c).is_none());
    }

    #[test]
    fn code_from_elsewhere_designs_and_pulse_become_notes() {
        let mit = (
            "LICENSE",
            "MIT License\n\nPermission is hereby granted, free of charge",
        );
        let gpl_header = "// This program is free software: you can redistribute it and/or modify\n// it under the terms of the GNU General Public License as published by\n// the Free Software Foundation.\nfn x() {}";
        let data = archive(&[
            mit,
            ("src/vendor/parser.rs", gpl_header),
            (
                "Source/Game.cpp",
                "// Copyright Epic Games, Inc. All Rights Reserved.\nint main() {}",
            ),
            (
                "Assets/Plugins/Fancy/License.txt",
                "This asset is governed by the Asset Store Terms of Service and EULA.",
            ),
            (
                "Assets/Shop.cs",
                "int premiumCurrency = 0; // hard_currency\nvoid SkipTimer() {}\nvar vip_level = 2;",
            ),
            (
                "Assets/Pulse.cs",
                "var token = Environment.GetEnvironmentVariable(\"SLOP_PULSE_TOKEN\"); Post(\"start\");",
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let notices: Vec<_> = report
            .licence
            .notices
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(notices, ["epic_games", "gpl", "unity_asset_store"]);
        assert!(report.licence.kinds.contains(&"notice:gpl".to_string()));
        let design: Vec<_> = report.design.iter().map(|h| h.rule.as_str()).collect();
        assert_eq!(design, ["energy_skip", "premium_currency", "vip_tiers"]);
        assert_eq!(report.pulse, "start_only");

        // The same GPL code in a GPL project is fine.
        let gpl = (
            "LICENSE",
            "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007",
        );
        let data = archive(&[gpl, ("src/parser.rs", gpl_header)]);
        let (report, _) = scan_archive(&data[..], "GPL-3.0-or-later", true).unwrap();
        assert!(report.licence.notices.is_empty());
        assert_eq!(report.pulse, "none");
    }

    #[test]
    fn closed_source_cg_is_a_licence_problem_but_unity_shaders_are_not() {
        let licence = (
            "LICENSE",
            "MIT License\n\nPermission is hereby granted, free of charge",
        );
        let data = archive(&[
            licence,
            (
                "src/render.cpp",
                "#include <GL/gl.h>\n#include <Cg/cgGL.h>\nint main() {}",
            ),
            (
                "CMakeLists.txt",
                "find_package(Cg REQUIRED)\ntarget_link_libraries(game -lCgGL)",
            ),
            (
                "Assets/Water.shader",
                "CGPROGRAM\n#pragma vertex vert\nENDCG",
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let closed: Vec<_> = report
            .licence
            .closed
            .iter()
            .map(|h| (h.rule.as_str(), h.count))
            .collect();
        assert_eq!(closed, [("nvidia_cg", 2)]);
        assert!(
            report
                .licence
                .kinds
                .contains(&"closed:nvidia_cg".to_string())
        );

        let data = archive(&[licence, ("Assets/Water.shader", "CGPROGRAM\nENDCG")]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        assert!(report.licence.closed.is_empty());
    }

    #[test]
    fn saves_outside_slops_save_folder_become_notes() {
        let mit = (
            "LICENSE",
            "MIT License\n\nPermission is hereby granted, free of charge",
        );
        let data = archive(&[
            mit,
            (
                "Scripts/Save.cs",
                "var dir = Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments);\nvar p = Application.persistentDataPath;",
            ),
            (
                "save.gd",
                "var f = FileAccess.open(\"user://save.json\", FileAccess.WRITE)",
            ),
            (
                "src/save.rs",
                "let home = std::env::var(\"HOME\")?;\nlet d = dirs::data_dir();",
            ),
            ("main.js", "const p = app.getPath('userData');"),
            ("game.py", "p = Path.home() / '.mygame'"),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let rules: Vec<_> = report.saves.iter().map(|h| h.rule.as_str()).collect();
        assert_eq!(
            rules,
            [
                "dotnet_folders",
                "electron_paths",
                "home_env",
                "home_python",
                "rust_dirs",
                "unity_data_path"
            ],
            "Godot's user:// isn't a note"
        );
        assert_eq!(report.save_folder, "elsewhere");
        let langs: Vec<_> = report
            .code_files
            .iter()
            .map(|h| (h.label.as_str(), h.count))
            .collect();
        assert_eq!(
            langs,
            [
                ("C#", 1),
                ("GDScript", 1),
                ("JavaScript", 1),
                ("Python", 1),
                ("Rust", 1)
            ]
        );
        let value = serde_json::to_value(&report).unwrap();
        // (The short licence text here isn't recognised; that's a licence note.)
        assert!(
            concerns(&value)
                .unwrap()
                .iter()
                .all(|k| k.starts_with("licence:")),
            "save notes don't hold updates"
        );

        // Code that reads SLOP_SAVE_DIR knows about Slop's save folder.
        let data = archive(&[
            mit,
            (
                "Scripts/Save.cs",
                "var dir = Environment.GetEnvironmentVariable(\"SLOP_SAVE_DIR\") ?? Application.persistentDataPath;",
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        assert!(report.saves.is_empty());
        assert_eq!(report.save_folder, "slop");
        let value = serde_json::to_value(&report).unwrap();
        assert!(value.get("saves").is_none());
        assert_eq!(value["save_folder"], "slop", "every checked release says");

        // Godot's user:// lands in the save folder; code that saves nothing says so.
        let data = archive(&[
            mit,
            (
                "save.gd",
                "var f = FileAccess.open(\"user://save.json\", FileAccess.WRITE)",
            ),
        ]);
        assert_eq!(
            scan_archive(&data[..], "MIT", true).unwrap().0.save_folder,
            "engine"
        );
        let data = archive(&[mit, ("main.gd", "func _ready():\n\tpass")]);
        assert_eq!(
            scan_archive(&data[..], "MIT", true).unwrap().0.save_folder,
            "none"
        );
    }

    #[test]
    fn code_from_outside_slop_becomes_notes() {
        let data = archive(&[
            (
                "LICENSE",
                "MIT License\n\nPermission is hereby granted, free of charge",
            ),
            (
                "net/Client.cs",
                "var asm = Assembly.Load(bytes);\nvar f = new BinaryFormatter();",
            ),
            (
                "net/client.gd",
                "var mod = load_resource_pack(path)\nvar v = bytes_to_var_with_objects(packet)\nvar peer = ENetMultiplayerPeer.new()",
            ),
            ("mods/run.lua", "local f = loadstring(body)"),
            (
                "tools/train.py",
                "model.eval()\nresult = re.compile('x').match(s)",
            ),
            (
                "web/app.js",
                "const m = /a/.exec(s);\nconst ws = new WebSocket(url);",
            ),
            (
                "pulse.py",
                "import urllib.request\nurl = os.environ['SLOP_PULSE_URL']",
            ),
            (
                "web/package-lock.json",
                r#"{"packages":{"node_modules/electron-updater":{"version":"6.0.0"}}}"#,
            ),
        ]);
        let (report, _) = scan_archive(&data[..], "MIT", true).unwrap();
        let rules: Vec<_> = report
            .code_loading
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(
            rules,
            [
                "load_assembly",
                "load_resource_pack",
                "load_script",
                "self_update",
                "unsafe_format"
            ],
            "model.eval() and regex .exec() aren't eval or exec"
        );
        let net: Vec<_> = report.network.iter().map(|h| h.rule.as_str()).collect();
        assert_eq!(
            net,
            ["http", "multiplayer"],
            "the Pulse snippet doesn't count"
        );
        assert_eq!(report.network[0].examples, ["web/app.js:2"]);
        let unsafe_format = report
            .code_loading
            .iter()
            .find(|h| h.rule == "unsafe_format")
            .unwrap();
        assert_eq!(unsafe_format.count, 2);
        assert_eq!(
            concerns(&serde_json::to_value(&report).unwrap())
                .unwrap()
                .into_iter()
                .filter(|k| k.starts_with("code:"))
                .count(),
            5
        );
    }

    #[test]
    fn archives_on_gitlab_and_gitea_hosts() {
        let commit = "a".repeat(40);
        let gitlab = archive_url(
            Kind::Gitlab,
            &"https://gitlab.com/group/sub/game.git".parse().unwrap(),
            &commit,
        )
        .unwrap();
        assert_eq!(
            gitlab.as_str(),
            format!(
                "https://gitlab.com/api/v4/projects/group%2Fsub%2Fgame/repository/archive.tar.gz?sha={commit}"
            )
        );
        let gitea = archive_url(
            Kind::Gitea,
            &"https://codeberg.org/ann/game/".parse().unwrap(),
            &commit,
        )
        .unwrap();
        assert_eq!(
            gitea.as_str(),
            format!("https://codeberg.org/api/v1/repos/ann/game/archive/{commit}.tar.gz")
        );
        assert!(
            archive_url(
                Kind::Gitlab,
                &"https://gitlab.com/lonely".parse().unwrap(),
                &commit
            )
            .is_none()
        );
        assert_eq!(
            archive_url(
                Kind::Bitbucket,
                &"https://bitbucket.org/a/b".parse().unwrap(),
                &commit
            )
            .unwrap()
            .as_str(),
            format!("https://bitbucket.org/a/b/get/{commit}.tar.gz"),
            "Bitbucket archives are read since 2026-10-21"
        );
    }

    #[test]
    fn licences_are_recognised() {
        assert_eq!(
            identify_licence(
                "MIT License\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software... The above copyright notice and this permission notice shall be included in all"
            ),
            Some("MIT")
        );
        assert_eq!(
            identify_licence(
                "                                 Apache License\n                           Version 2.0, January 2004"
            ),
            Some("Apache-2.0")
        );
        assert_eq!(
            identify_licence("GNU GENERAL PUBLIC LICENSE\n Version 3, 29 June 2007"),
            Some("GPL-3.0")
        );
        assert_eq!(
            identify_licence("GNU GENERAL PUBLIC LICENSE\n Version 2, June 1991"),
            Some("GPL-2.0")
        );
        assert_eq!(identify_licence("All rights reserved."), None);
        assert_eq!(
            expression_ids("(MIT OR Apache-2.0) AND GPL-2.0-only WITH Classpath-exception-2.0"),
            ["MIT", "Apache-2.0", "GPL-2.0-only"]
        );
        assert_eq!(family("GPL-3.0-or-later"), family("GPL-3.0"));
    }

    #[test]
    fn more_package_lists_give_packages() {
        let names = |file: &str, text: &str| -> Vec<String> {
            parse_lockfile(file, text)
                .into_iter()
                .map(|p| format!("{} {}@{}", p.ecosystem, p.name, p.version))
                .collect()
        };
        let pnpm = "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies:\n      lodash:\n        version: 4.17.21\npackages:\n  lodash@4.17.21:\n    resolution: {integrity: x}\n  '@stripe/stripe-js@4.1.0':\n    resolution: {integrity: y}\n  react-dom@19.0.0(react@19.0.0):\n    resolution: {integrity: z}\nsnapshots:\n  other@1.0.0: {}\n";
        assert_eq!(
            names("pnpm-lock.yaml", pnpm),
            [
                "npm lodash@4.17.21",
                "npm @stripe/stripe-js@4.1.0",
                "npm react-dom@19.0.0"
            ]
        );
        assert_eq!(
            names(
                "pnpm-lock.yaml",
                "lockfileVersion: 5.4\npackages:\n  /left-pad/1.3.0:\n    dev: false\n"
            ),
            ["npm left-pad@1.3.0"]
        );
        let gradle = "# comment\ncom.android.billingclient:billing:7.0.0=releaseRuntimeClasspath\njunit:junit:4.13.2=testCompileClasspath,testRuntimeClasspath\nempty=\n";
        assert_eq!(
            names("gradle.lockfile", gradle),
            ["Maven com.android.billingclient:billing@7.0.0"]
        );
        let unity = r#"{"dependencies": {"com.unity.purchasing": {"version": "4.12.2", "source": "registry"}, "com.unity.modules.ui": {"version": "1.0.0", "source": "builtin"}}}"#;
        assert_eq!(
            names("packages-lock.json", unity),
            ["Unity com.unity.purchasing@4.12.2"]
        );
        let godot = "[plugin]\n\nname=\"GodotGooglePlayBilling\"\ndescription=\"x\"\nversion=\"2.1.0\"\nscript=\"plugin.gd\"\n";
        assert_eq!(
            names("plugin.cfg", godot),
            ["Godot GodotGooglePlayBilling@2.1.0"]
        );
        // The payment check sees them.
        assert!(
            parse_lockfile("packages-lock.json", unity)
                .iter()
                .any(is_payment_package)
        );
        assert!(
            parse_lockfile("plugin.cfg", godot)
                .iter()
                .any(is_payment_package)
        );
        assert!(
            parse_lockfile("gradle.lockfile", gradle)
                .iter()
                .any(is_payment_package)
        );
    }

    #[test]
    fn lockfiles_give_packages() {
        let cargo = "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n[[package]]\nname = \"mygame\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            parse_lockfile("Cargo.lock", cargo),
            [Package {
                ecosystem: "crates.io".into(),
                name: "serde".into(),
                version: "1.0.0".into()
            }]
        );
        let npm = r#"{"lockfileVersion":3,"packages":{"":{"name":"g"},"node_modules/lodash":{"version":"4.17.20"},"node_modules/a/node_modules/@scope/b":{"version":"1.0.0"},"node_modules/vitest":{"version":"1.0.0","dev":true}}}"#;
        let mut names: Vec<_> = parse_lockfile("package-lock.json", npm)
            .into_iter()
            .map(|p| p.name)
            .collect();
        names.sort();
        assert_eq!(names, ["@scope/b", "lodash"]);
        let yarn = "# yarn\n\n\"@babel/core@^7.0.0\", \"@babel/core@^7.1.0\":\n  version \"7.2.0\"\n  resolved \"x\"\n\nleft-pad@1.0.0:\n  version \"1.0.0\"\n";
        let got: Vec<_> = parse_lockfile("yarn.lock", yarn)
            .into_iter()
            .map(|p| format!("{}@{}", p.name, p.version))
            .collect();
        assert_eq!(got, ["@babel/core@7.2.0", "left-pad@1.0.0"]);
        let go = "module x\n\nrequire github.com/a/b v1.2.3\nrequire (\n\tgolang.org/x/net v0.1.0 // indirect\n)\n";
        assert_eq!(parse_lockfile("go.mod", go).len(), 2);
        assert_eq!(
            parse_lockfile(
                "requirements.txt",
                "requests[socks]==2.0.1 ; python_version>'3'\nflask>=2\n"
            )[0]
            .name,
            "requests"
        );
        let nuget = r#"{"dependencies":{"net8.0":{"Newtonsoft.Json":{"type":"Direct","resolved":"13.0.1"},"Game.Core":{"type":"Project"}}}}"#;
        assert_eq!(parse_lockfile("packages.lock.json", nuget).len(), 1);
    }

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
            let mut tar = tar::Builder::new(gz);
            for (path, text) in files {
                let mut h = tar::Header::new_gnu();
                h.set_size(text.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                tar.append_data(&mut h, format!("octo-game-abc/{path}"), text.as_bytes())
                    .unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap();
        }
        out
    }

    #[test]
    fn archives_give_licence_monetization_and_package_notes() {
        let data = archive(&[
            (
                "LICENSE",
                "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007",
            ),
            (
                "src/shop.cs",
                "// SPDX-License-Identifier: GPL-3.0-or-later\nusing UnityEngine.Purchasing;\nclass Shop { void OpenLootBox() {} string gemPack = \"gem pack\"; }",
            ),
            (
                "src/main.rs",
                "// SPDX-License-Identifier: MIT\nfn main() {}",
            ),
            ("node_modules/stripe/index.js", "api.stripe.com"),
            (
                "web/package-lock.json",
                r#"{"packages":{"node_modules/stripe":{"version":"12.0.0"}}}"#,
            ),
        ]);
        let (report, packages) = scan_archive(&data[..], "MIT", true).unwrap();
        assert_eq!(report.licence.files[0].spdx, Some("GPL-3.0"));
        assert!(
            report
                .licence
                .problems
                .iter()
                .any(|p| p.contains("says GPL-3.0, but the store page says MIT")),
            "{:?}",
            report.licence.problems
        );
        assert!(
            report
                .licence
                .problems
                .iter()
                .any(|p| p.contains("1 source file says GPL-3.0-or-later")),
            "{:?}",
            report.licence.problems
        );
        let rules: Vec<_> = report
            .monetization
            .iter()
            .map(|h| h.rule.as_str())
            .collect();
        assert_eq!(
            rules,
            ["payment_library", "premium_currency", "unity_iap"],
            "node_modules isn't read; LootBox isn't a word boundary"
        );
        assert_eq!(packages.len(), 1);
        assert_eq!(report.lockfiles, ["web/package-lock.json"]);

        let (clean, _) = scan_archive(&archive(&[("LICENSE", "Permission is hereby granted, free of charge, to any person obtaining a copy. The above copyright notice and this permission notice shall be included")])[..], "MIT", true).unwrap();
        assert!(clean.licence.problems.is_empty() && clean.monetization.is_empty());
        let (missing, _) = scan_archive(
            &archive(&[("README.md", "hi")])[..],
            "Proprietary-1.0",
            true,
        )
        .unwrap();
        assert_eq!(
            missing.licence.problems.len(),
            2,
            "no file, and not OSI-approved"
        );
    }
}
