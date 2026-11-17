// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the source check looks for: which files it reads, and the patterns in them.

use std::sync::LazyLock;

use regex::Regex;

use crate::Package;

/// Folders whose contents aren't the maker's source (installed packages, build output, engine
/// caches).
pub(crate) const SKIP_DIRS: &[&str] = &[
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

pub(crate) const SOURCE_EXTS: &[&str] = &[
    "rs", "go", "cs", "gd", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "lua", "c", "cc", "cpp",
    "h", "hpp", "java", "kt", "swift", "html", "htm", "vue", "svelte", "dart", "rb", "php",
];

/// Languages for `code_files`, by extension.
pub(crate) fn language(ext: &str) -> Option<&'static str> {
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

pub(crate) struct Rule {
    pub(crate) rule: &'static str,
    pub(crate) label: &'static str,
    pub(crate) re: Regex,
}

pub(crate) static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
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

/// Code from outside SLAPorSLOP: loading code at run time, self-updaters, and data formats that can
/// rebuild arbitrary objects. (rule, label, file extensions it applies to, pattern)
pub(crate) static CODE_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(
    || {
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
                    "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "lua", "gd", "html", "htm",
                    "vue", "svelte", "rb", "php", "java", "kt",
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
                "Updates itself outside SLAPorSLOP",
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
    },
);

/// Networking code, by engine and language. (rule, label, file extensions, pattern)
pub(crate) static NET_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(|| {
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
/// DESIGN §10). Godot's `user://` isn't listed: the SLAPorSLOP app points it at the save folder.
/// (rule, label, file extensions, pattern)
pub(crate) static SAVE_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(
    || {
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
                    "gd", "cs", "rs", "c", "cc", "cpp", "h", "hpp", "lua", "java", "kt", "py",
                    "go", "js", "mjs", "cjs", "ts", "rb", "php", "dart",
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
    },
);

/// Licence texts and vendor notices in files: (rule, label, pattern, declared licence families
/// that make it fine; empty: never fine to publish).
pub(crate) static NOTICE_RULES: LazyLock<Vec<(Rule, &'static [&'static str])>> = LazyLock::new(
    || {
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
    },
);

/// Closed-source libraries, found where code includes or links them (not shader source: Unity's
/// `CGPROGRAM` blocks are compiled by Unity, without NVIDIA's runtime).
pub(crate) static CLOSED_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
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
pub(crate) static DESIGN_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
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

/// Self-updaters: they replace the app with whatever their server sends, bypassing SLAPorSLOP.
pub(crate) const UPDATER_PACKAGES: &[(&str, &str)] = &[
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
pub(crate) const PAYMENT_PACKAGES: &[(&str, &str)] = &[
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

pub(crate) fn is_payment_package(p: &Package) -> bool {
    in_list(PAYMENT_PACKAGES, p)
}

pub(crate) fn in_list(list: &[(&str, &str)], p: &Package) -> bool {
    list.iter().any(|(eco, name)| {
        *eco == p.ecosystem
            && match name.strip_suffix('*') {
                Some(prefix) => p.name.starts_with(prefix),
                None => p.name.eq_ignore_ascii_case(name),
            }
    })
}
