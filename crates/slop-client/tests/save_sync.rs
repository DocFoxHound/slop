// SPDX-License-Identifier: AGPL-3.0-or-later
//! Save sync between two computers, through a fake SLAPorSLOP that keeps saves the way slop-api's
//! save_sync.rs does (generations; an upload on top of an old one is refused).

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use slop_client::{
    api::{ApiClient, MemoryTokenStore},
    save_sync::{self, Outcome, State as SyncState},
};

#[derive(Default)]
struct Store {
    /// (generation, device, body)
    newest: Mutex<Option<(i32, String, Vec<u8>)>>,
    seen: Mutex<Vec<Vec<u8>>>,
}

async fn fake() -> (String, Arc<Store>) {
    let store = Arc::new(Store::default());
    let app = Router::new()
        .route(
            "/v1/me/saves/{slug}",
            get(|State(s): State<Arc<Store>>| async move {
                match s.newest.lock().unwrap().clone() {
                    None => StatusCode::NOT_FOUND.into_response(),
                    Some((g, d, b)) => (
                        [("x-slop-generation", g.to_string()), ("x-slop-device", d)],
                        b,
                    )
                        .into_response(),
                }
            })
            .put(
                |State(s): State<Arc<Store>>, headers: HeaderMap, body: Bytes| async move {
                    let base: i32 = headers["x-slop-base"].to_str().unwrap().parse().unwrap();
                    let device = headers["x-slop-device"].to_str().unwrap().to_string();
                    let mut newest = s.newest.lock().unwrap();
                    let current = newest.as_ref().map_or(0, |n| n.0);
                    if base != current {
                        return StatusCode::CONFLICT.into_response();
                    }
                    s.seen.lock().unwrap().push(body.to_vec());
                    *newest = Some((current + 1, device, body.to_vec()));
                    axum::Json(serde_json::json!({ "generation": current + 1 })).into_response()
                },
            ),
        )
        .with_state(store.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, store)
}

/// A computer: its save folder, copies, scratch space and sync state.
struct Computer {
    saves: PathBuf,
    copies: PathBuf,
    scratch: PathBuf,
    state: SyncState,
}

fn computer() -> Computer {
    let root = std::env::temp_dir().join(format!("slop-sync-pc-{}", rand::random::<u64>()));
    let c = Computer {
        saves: root.join("saves"),
        copies: root.join("copies"),
        scratch: root.join("scratch"),
        state: SyncState::load(&root.join("sync.json")),
    };
    for d in [&c.saves, &c.copies, &c.scratch] {
        std::fs::create_dir_all(d).unwrap();
    }
    c
}

#[tokio::test]
async fn saves_follow_the_player_and_never_overwrite_silently() {
    let (base, store) = fake().await;
    let api = ApiClient::new(base.parse().unwrap(), Arc::new(MemoryTokenStore::default())).unwrap();
    let key = save_sync::derive_key(&[9u8; 32]);
    let (mut home, mut laptop) = (computer(), computer());
    let pull = |c: &mut Computer, force: bool| {
        let api = api.clone();
        let (saves, copies, scratch) = (c.saves.clone(), c.copies.clone(), c.scratch.clone());
        let mut state = std::mem::take(&mut c.state);
        async move {
            let out = save_sync::pull(
                &api, &key, "pong", &saves, &copies, &scratch, &mut state, force,
            )
            .await
            .unwrap();
            (out, state)
        }
    };

    // The home PC plays and sends its save; SLAPorSLOP only ever sees ciphertext.
    std::fs::write(home.saves.join("pong.json"), r#"{"wins":3}"#).unwrap();
    let out = save_sync::push(&api, &key, "pong", &home.saves, &mut home.state, false)
        .await
        .unwrap();
    assert_eq!(out, Outcome::Sent { generation: 1 });
    assert!(!String::from_utf8_lossy(&store.seen.lock().unwrap()[0]).contains("wins"));
    // Nothing changed: nothing sent.
    assert_eq!(
        save_sync::push(&api, &key, "pong", &home.saves, &mut home.state, false)
            .await
            .unwrap(),
        Outcome::UpToDate
    );

    // The laptop, never played, gets it before launching.
    let (out, state) = pull(&mut laptop, false).await;
    laptop.state = state;
    assert_eq!(out, Outcome::Fetched { generation: 1 });
    assert_eq!(
        std::fs::read_to_string(laptop.saves.join("pong.json")).unwrap(),
        r#"{"wins":3}"#
    );

    // The laptop plays on and sends; home fetches it next time.
    std::fs::write(laptop.saves.join("pong.json"), r#"{"wins":5}"#).unwrap();
    assert_eq!(
        save_sync::push(&api, &key, "pong", &laptop.saves, &mut laptop.state, false)
            .await
            .unwrap(),
        Outcome::Sent { generation: 2 }
    );
    let (out, state) = pull(&mut home, false).await;
    home.state = state;
    assert_eq!(out, Outcome::Fetched { generation: 2 });
    assert_eq!(
        std::fs::read_to_string(home.saves.join("pong.json")).unwrap(),
        r#"{"wins":5}"#
    );
    assert!(
        !slop_client::saves::copies(&home.copies).is_empty(),
        "the old saves were copied first"
    );

    // Both play offline: neither overwrites the other.
    std::fs::write(home.saves.join("pong.json"), r#"{"wins":6}"#).unwrap();
    std::fs::write(laptop.saves.join("pong.json"), r#"{"wins":9}"#).unwrap();
    assert_eq!(
        save_sync::push(&api, &key, "pong", &laptop.saves, &mut laptop.state, false)
            .await
            .unwrap(),
        Outcome::Sent { generation: 3 }
    );
    assert!(matches!(
        save_sync::push(&api, &key, "pong", &home.saves, &mut home.state, false)
            .await
            .unwrap(),
        Outcome::BothChanged { .. }
    ));
    let (out, state) = pull(&mut home, false).await;
    home.state = state;
    assert!(matches!(out, Outcome::BothChanged { .. }));
    assert_eq!(
        std::fs::read_to_string(home.saves.join("pong.json")).unwrap(),
        r#"{"wins":6}"#,
        "kept"
    );

    // The player chooses home's saves: they go on top.
    assert_eq!(
        save_sync::push(&api, &key, "pong", &home.saves, &mut home.state, true)
            .await
            .unwrap(),
        Outcome::Sent { generation: 4 }
    );
    let (out, state) = pull(&mut laptop, true).await;
    laptop.state = state;
    assert_eq!(out, Outcome::Fetched { generation: 4 });
    assert_eq!(
        std::fs::read_to_string(laptop.saves.join("pong.json")).unwrap(),
        r#"{"wins":6}"#
    );

    // A computer without the right key can't read it.
    let other = save_sync::derive_key(&[1u8; 32]);
    let mut stranger = computer();
    let mut state = std::mem::take(&mut stranger.state);
    assert!(
        save_sync::pull(
            &api,
            &other,
            "pong",
            &stranger.saves,
            &stranger.copies,
            &stranger.scratch,
            &mut state,
            false
        )
        .await
        .is_err()
    );
}
