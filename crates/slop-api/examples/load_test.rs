// SPDX-License-Identifier: AGPL-3.0-or-later
//! Load test: how much the API takes (docs/LOAD-TEST.md). Never against a real Slop.
//!
//! Starts the throwaway Slop (`e2e_server`, release build) filled by the simulation, then for
//! each kind of request runs a rising number of simultaneous clients for a few seconds and
//! prints requests a second, latency percentiles and errors as a Markdown table.
//!
//! cargo build --release -p slop-api --examples
//! DATABASE_URL=postgres://slop:slop@127.0.0.1:5433/slop target/release/examples/load_test

use std::{
    io::BufRead,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use serde_json::Value;

/// Seconds each step runs, and the client counts tried.
const STEP: Duration = Duration::from_secs(8);
const CLIENTS: &[usize] = &[1, 10, 50, 100];

#[derive(Clone)]
enum Kind {
    Get(Vec<String>),
    /// Pulse heartbeats with these tokens.
    Heartbeat(String, Vec<String>),
}

struct Scenario {
    name: &'static str,
    kind: Kind,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bin = std::env::current_exe()?.with_file_name("e2e_server");
    let mut server = Command::new(&bin)
        .env("SLOP_E2E_SIMULATE", "400,1500")
        .env("RUST_LOG", "error")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", bin.display()))?;
    let mut line = String::new();
    std::io::BufReader::new(server.stdout.take().unwrap()).read_line(&mut line)?;
    let info: Value = serde_json::from_str(&line).context("the test server's first line")?;
    let base = info["url"].as_str().unwrap().to_string();
    let cookie = format!(
        "{}={}",
        info["cookie_name"].as_str().unwrap(),
        info["cookies"]["player"].as_str().unwrap()
    );
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(200)
        .build()?;

    // What the store holds, to ask for.
    let apps: Value = client
        .get(format!("{base}/v1/apps?sort=popular&limit=100"))
        .send()
        .await?
        .json()
        .await?;
    let slugs: Vec<String> = apps["apps"]
        .as_array()
        .context("the store's apps")?
        .iter()
        .filter_map(|a| a["slug"].as_str().map(String::from))
        .collect();
    anyhow::ensure!(slugs.len() >= 20, "the simulation made too few apps");
    let pick = |n: usize, f: &dyn Fn(&str) -> String| -> Vec<String> {
        slugs.iter().take(n).map(|s| f(s)).collect()
    };
    let updates: Vec<String> = (0..20)
        .map(|i| {
            let list: Vec<&str> = slugs.iter().skip(i).take(20).map(String::as_str).collect();
            format!("{base}/v1/library/updates?apps={}", list.join(","))
        })
        .collect();
    // Pulse: a launch token per pretend player session.
    let mut tokens = Vec::new();
    let mut pulse_url = String::new();
    for slug in slugs.iter().take(50) {
        let res = client
            .post(format!("{base}/v1/apps/{slug}/launch?platform=windows"))
            .header("cookie", &cookie)
            .send()
            .await?;
        if let Ok(body) = res.json::<Value>().await
            && let Some(t) = body["token"].as_str()
        {
            pulse_url = body["pulse_url"].as_str().unwrap_or_default().to_string();
            let start = client
                .post(format!("{pulse_url}/start"))
                .json(&serde_json::json!({ "token": t }))
                .send()
                .await?;
            if start.status().is_success() {
                tokens.push(t.to_string());
            }
        }
    }

    let scenarios = vec![
        Scenario {
            name: "Website (index.html)",
            kind: Kind::Get(vec![format!("{base}/")]),
        },
        Scenario {
            name: "Store front (popular)",
            kind: Kind::Get(vec![format!("{base}/v1/apps?sort=popular")]),
        },
        Scenario {
            name: "Store page",
            kind: Kind::Get(pick(50, &|s| format!("{base}/v1/apps/{s}"))),
        },
        Scenario {
            name: "Search",
            kind: Kind::Get(
                ["castle", "space", "puzzle", "racing", "farm", "dungeon"]
                    .iter()
                    .map(|q| format!("{base}/v1/apps?q={q}"))
                    .collect(),
            ),
        },
        Scenario {
            name: "Library update check (20 apps)",
            kind: Kind::Get(updates),
        },
        Scenario {
            name: "Pulse heartbeat",
            kind: Kind::Heartbeat(format!("{pulse_url}/heartbeat"), tokens),
        },
    ];

    println!("| Request | Clients | Requests/s | p50 ms | p95 ms | p99 ms | Errors |");
    println!("|---|---|---|---|---|---|---|");
    for s in &scenarios {
        if let Kind::Heartbeat(_, t) = &s.kind
            && t.is_empty()
        {
            println!("| {} | — | no launch tokens | | | | |", s.name);
            continue;
        }
        for &clients in CLIENTS {
            let (count, errors, mut times) = run(&client, &s.kind, clients).await;
            times.sort_unstable();
            let pct = |p: f64| {
                times
                    .get(((times.len() as f64 * p) as usize).min(times.len().saturating_sub(1)))
                    .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
            };
            println!(
                "| {} | {clients} | {:.0} | {:.1} | {:.1} | {:.1} | {errors} |",
                s.name,
                count as f64 / STEP.as_secs_f64(),
                pct(0.5),
                pct(0.95),
                pct(0.99),
            );
        }
    }

    drop(server.stdin.take());
    server.wait()?;
    Ok(())
}

/// `clients` loops sending requests for `STEP`. Returns (requests, errors, latencies).
async fn run(
    client: &reqwest::Client,
    kind: &Kind,
    clients: usize,
) -> (usize, usize, Vec<Duration>) {
    let until = Instant::now() + STEP;
    let kind = Arc::new(kind.clone());
    let mut tasks = Vec::new();
    for c in 0..clients {
        let (client, kind) = (client.clone(), kind.clone());
        tasks.push(tokio::spawn(async move {
            let (mut n, mut errors, mut times) = (0usize, 0usize, Vec::new());
            let mut i = c;
            while Instant::now() < until {
                let started = Instant::now();
                let res = match &*kind {
                    Kind::Get(urls) => client.get(&urls[i % urls.len()]).send().await,
                    Kind::Heartbeat(url, tokens) => {
                        client
                            .post(url)
                            .json(&serde_json::json!({ "token": tokens[i % tokens.len()] }))
                            .send()
                            .await
                    }
                };
                let ok = match res {
                    Ok(r) => {
                        let ok = r.status().is_success();
                        let _ = r.bytes().await;
                        ok
                    }
                    Err(_) => false,
                };
                times.push(started.elapsed());
                n += 1;
                if !ok {
                    errors += 1;
                }
                i += clients;
            }
            (n, errors, times)
        }));
    }
    let (mut n, mut errors, mut times) = (0, 0, Vec::new());
    for t in tasks {
        let (a, b, c) = t.await.unwrap();
        n += a;
        errors += b;
        times.extend(c);
    }
    (n, errors, times)
}
