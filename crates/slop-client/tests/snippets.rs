// SPDX-License-Identifier: AGPL-3.0-or-later
//! The publishing guide's code for other engines (examples/snippets), run against the real
//! Slop Link: each saves and loads, lists mods from its own mods folder and SLOP_MODS, and asks
//! for a mod. C# and C++ run where `dotnet` and `g++` are installed; skipped otherwise.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    time::Duration,
};

use slop_client::link::{Link, ModResult, Outcome};

fn snippets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/snippets")
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slop-snippet-{tag}-{}", rand::random::<u64>()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A save folder, mods (one in the game's own folder and in SLOP_MODS, one only in SLOP_MODS),
/// and a Slop Link that says yes. Returns the environment for the program and what to check.
struct World {
    root: PathBuf,
    link: Link,
    link_dir: PathBuf,
    env: Vec<(&'static str, String)>,
    own_mods: PathBuf,
}

fn world(tag: &str) -> World {
    let root = tempdir(tag);
    let own_mods = root.join("game/mods");
    std::fs::create_dir_all(own_mods.join("neon")).unwrap();
    for m in ["neon", "rainbow"] {
        std::fs::create_dir_all(root.join(format!("slop/mods/{m}/files"))).unwrap();
    }
    let slop_mods = std::env::join_paths([
        root.join("slop/mods/neon/files"),
        root.join("slop/mods/rainbow/files"),
    ])
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let link = Link::start_every(root.join("link"), Duration::from_millis(50), move |r| {
        let _ = tx.send(r.clone());
    })
    .unwrap();
    let answering = link.clone();
    std::thread::spawn(move || {
        while let Ok(request) = rx.recv() {
            answering.finish(
                &request.id,
                Outcome::Done {
                    mods: request
                        .mods
                        .iter()
                        .map(|m| ModResult {
                            slug: m.slug.clone(),
                            result: "added",
                            version: m.version.clone(),
                        })
                        .collect(),
                    folders: vec![],
                },
            );
        }
    });
    let link_dir = link.open("snippet-game").unwrap();
    World {
        env: vec![
            ("SLOP_SAVE_DIR", root.join("saves").display().to_string()),
            ("SLOP_MODS", slop_mods.to_string_lossy().into_owned()),
            ("SLOP_LINK", link_dir.display().to_string()),
            ("OWN_MODS", own_mods.display().to_string()),
        ],
        root,
        link,
        link_dir,
        own_mods,
    }
}

/// What every program prints when it works.
fn check(world: World, out: std::process::Output) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stdout}\n{stderr}");
    let saves = world.root.join("saves");
    assert!(stdout.contains("save ok"), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(saves.join("game.json")).unwrap(),
        r#"{"level": 3}"#
    );
    assert!(!saves.join("game.json.tmp").exists());
    assert!(
        stdout.contains("mods 2"),
        "own neon + SLOP_MODS rainbow, once each: {stdout}"
    );
    assert!(stdout.contains("status done"), "{stdout}");
    assert!(world.own_mods.exists());
    world.link.close(&world.link_dir);
    let _ = std::fs::remove_dir_all(&world.root);
}

#[test]
fn python() {
    let w = world("py");
    let script = r#"
import os, sys
sys.path.insert(0, sys.argv[1])
import slop
slop.save_json("/nonexistent", "game.json", {"level": 3})
with open(os.path.join(os.environ["SLOP_SAVE_DIR"], "game.json"), "w") as f: f.write('{"level": 3}')
assert slop.load_json("/nonexistent", "game.json") == {"level": 3}
assert slop.load_json("/nonexistent", "missing.json") == {}
print("save ok")
print("mods", len(slop.mod_dirs(os.environ["OWN_MODS"])))
answer = slop.request_mods([{"slug": "neon", "version": "1.0.0"}], "Test server", 20)
print("status", answer["status"])
"#;
    let out = Command::new("python3")
        .args(["-I", "-c", script])
        .arg(snippets())
        .envs(w.env.iter().cloned())
        .output()
        .unwrap();
    check(w, out);
}

#[test]
fn rust() {
    let w = world("rs");
    // The snippet compiled into a tiny program, so its environment is its own.
    let dir = tempdir("rs-build");
    std::fs::write(
        dir.join("main.rs"),
        format!(
            r##"#[path = {:?}] mod slop;
fn main() {{
    let fallback = std::path::Path::new("/nonexistent");
    slop::save(fallback, "game.json", br#"{{"level": 3}}"#).unwrap();
    assert_eq!(slop::load(fallback, "game.json").unwrap(), br#"{{"level": 3}}"#);
    assert!(slop::load(fallback, "missing.json").is_none());
    println!("save ok");
    let own = std::env::var("OWN_MODS").unwrap();
    println!("mods {{}}", slop::mod_dirs(Some(std::path::Path::new(&own))).len());
    let answer = slop::request_mods(serde_json::json!([{{"slug": "neon"}}]), "Test server", std::time::Duration::from_secs(20));
    println!("status {{}}", answer["status"].as_str().unwrap());
}}"##,
            snippets().join("slop.rs").canonicalize().unwrap()
        ),
    )
    .unwrap();
    // Built with this workspace's serde_json, from the target folder's dependency files.
    let deps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/deps");
    // The newest one: an older build (another toolchain or machine) may have left others.
    let serde_json = std::fs::read_dir(&deps)
        .unwrap()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("libserde_json-") && n.ends_with(".rlib")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
        .expect("serde_json in target/debug/deps");
    let exe = dir.join("snippet");
    let build = Command::new("rustc")
        .args(["--edition", "2021", "-o"])
        .arg(&exe)
        .arg(dir.join("main.rs"))
        .arg("--extern")
        .arg(format!("serde_json={}", serde_json.display()))
        .arg("-L")
        .arg(&deps)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&exe)
        .envs(w.env.iter().cloned())
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    check(w, out);
}

#[test]
fn csharp() {
    if Command::new("dotnet").arg("--version").output().is_err() {
        eprintln!("skipping: no dotnet");
        return;
    }
    let w = world("cs");
    let dir = tempdir("cs-build");
    std::fs::write(
        dir.join("snippet.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><Nullable>disable</Nullable><ImplicitUsings>disable</ImplicitUsings></PropertyGroup>
</Project>"#,
    )
    .unwrap();
    std::fs::copy(snippets().join("Slop.cs"), dir.join("Slop.cs")).unwrap();
    std::fs::write(
        dir.join("Program.cs"),
        r#"using System;
using System.IO;
var fallback = "/nonexistent";
Slop.Save(fallback, "game.json", "{\"level\": 2}");
Slop.Save(fallback, "game.json", "{\"level\": 3}");
if (Slop.Load(fallback, "game.json") != "{\"level\": 3}" || Slop.Load(fallback, "missing.json") != null) return 1;
Console.WriteLine("save ok");
Console.WriteLine("mods " + Slop.ModDirs(Environment.GetEnvironmentVariable("OWN_MODS")).Count);
var answer = await Slop.RequestMods(new[] { ("neon", (string)null) }, "Test \"server\"", 20);
Console.WriteLine("status " + Slop.Status(answer));
return 0;
"#,
    )
    .unwrap();
    let out = Command::new("dotnet")
        .args(["run", "--project"])
        .arg(&dir)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .envs(w.env.iter().cloned())
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    check(w, out);
}

#[test]
fn cpp() {
    if Command::new("g++").arg("--version").output().is_err() {
        eprintln!("skipping: no g++");
        return;
    }
    let w = world("cpp");
    let dir = tempdir("cpp-build");
    std::fs::write(
        dir.join("main.cpp"),
        r#"#include "slop.hpp"
#include <iostream>
int main() {
    std::filesystem::path fallback = "/nonexistent";
    if (!slop::save(fallback, "game.json", "{\"level\": 3}")) return 1;
    if (slop::load(fallback, "game.json").value_or("") != "{\"level\": 3}" || slop::load(fallback, "missing.json")) return 2;
    std::cout << "save ok\n";
    std::cout << "mods " << slop::mod_dirs(slop::env("OWN_MODS")).size() << "\n";
    std::string answer = slop::request_mods({{"neon", "1.0.0"}}, "Test \"server\"", std::chrono::seconds(20));
    std::cout << "status " << slop::status(answer) << "\n";
}
"#,
    )
    .unwrap();
    let exe = dir.join("snippet");
    let build = Command::new("g++")
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(snippets())
        .arg(dir.join("main.cpp"))
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&exe)
        .envs(w.env.iter().cloned())
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    check(w, out);
}

/// The guide shows each file exactly as tested.
#[test]
fn the_guide_shows_the_tested_code() {
    let guide = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/public/publishing.md"),
    )
    .unwrap();
    for file in ["slop.py", "slop.rs", "Slop.cs", "slop.hpp"] {
        let code = std::fs::read_to_string(snippets().join(file)).unwrap();
        assert!(
            guide.contains(code.trim_end()),
            "publishing.md is missing {file} as it stands"
        );
    }
}
