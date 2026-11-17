// SPDX-License-Identifier: AGPL-3.0-or-later
//! The packages lockfiles name, for the daily advisory watch (crates/slop-api/src/dep_watch.rs).

use serde_json::Value;

use crate::Package;

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

pub(crate) fn is_lockfile(name: &str) -> bool {
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
