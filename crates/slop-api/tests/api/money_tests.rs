// SPDX-License-Identifier: AGPL-3.0-or-later
//! Donations, Stripe checkout and webhooks, donation holds, bounties, offers.
//!
//! The harness here: a stateful fake Stripe (accounts, Checkout sessions, subscriptions,
//! invoices, refunds, application fees, saved cards, with failures on demand and a log of every
//! call), signed webhook events, and a few set-ups the tests share.

use crate::*;

mod bounties;
mod donations;
mod holds;
mod makers;
mod offers;
mod webhooks;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// One call Slop made to the fake Stripe.
#[derive(Clone, Debug)]
struct Call {
    method: String,
    /// Without the leading slash: `v1/subscriptions/sub_1`.
    path: String,
    /// The `Stripe-Account` header: the connected account it was made on.
    account: Option<String>,
    form: HashMap<String, String>,
    idempotency_key: Option<String>,
}

#[derive(Default)]
struct StripeState {
    calls: Vec<Call>,
    /// Checkout sessions by ID; `account` is the connected account each was made on.
    sessions: HashMap<String, Value>,
    accounts: HashMap<String, Value>,
    subscriptions: HashMap<String, Value>,
    invoices: HashMap<String, Value>,
    charges: HashMap<String, Value>,
    application_fees: Vec<Value>,
    /// (method, path prefix, status): answered with that error instead.
    failures: Vec<(String, String, u16)>,
    /// What new accounts look like: active unless a test says otherwise.
    new_account_status: Option<&'static str>,
    /// The status payment intents end in: succeeded unless a test says otherwise.
    intent_status: Option<&'static str>,
    next: u32,
}

/// A fake Stripe API on a local port.
#[derive(Clone)]
struct FakeStripe {
    base: String,
    state: Arc<Mutex<StripeState>>,
}

/// A Stripe account object in the given state: active, pending (still signing up) or
/// restricted (Stripe disabled it).
fn stripe_account(id: &str, status: &str) -> Value {
    match status {
        "active" => {
            json!({ "id": id, "charges_enabled": true, "details_submitted": true, "requirements": { "disabled_reason": null } })
        }
        "restricted" => {
            json!({ "id": id, "charges_enabled": false, "details_submitted": true, "requirements": { "disabled_reason": "requirements.past_due" } })
        }
        _ => {
            json!({ "id": id, "charges_enabled": false, "details_submitted": false, "requirements": { "disabled_reason": "requirements.pending_verification" } })
        }
    }
}

impl FakeStripe {
    async fn start() -> Self {
        let state: Arc<Mutex<StripeState>> = Arc::default();
        let handler = {
            let state = state.clone();
            move |method: Method,
                  uri: axum::http::Uri,
                  headers: axum::http::HeaderMap,
                  body: String| {
                let state = state.clone();
                async move { stripe_answer(&state, method, uri, headers, body) }
            }
        };
        let app = Router::new().fallback(handler);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeStripe {
            base: format!("http://{addr}/"),
            state,
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut StripeState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// Calls with this method whose path starts with `prefix`.
    fn calls(&self, method: &str, prefix: &str) -> Vec<Call> {
        self.with(|s| {
            s.calls
                .iter()
                .filter(|c| c.method == method && c.path.starts_with(prefix))
                .cloned()
                .collect()
        })
    }

    /// From now on, answers these calls with an error.
    fn fail(&self, method: &str, prefix: &str, status: u16) {
        self.with(|s| {
            s.failures
                .push((method.to_string(), prefix.to_string(), status))
        });
    }

    fn heal(&self) {
        self.with(|s| s.failures.clear());
    }

    fn session(&self, id: &str) -> Value {
        self.with(|s| s.sessions[id].clone())
    }

    /// Changes a session as Stripe would after the donor acts on it.
    fn patch_session(&self, id: &str, patch: Value) {
        self.with(|s| {
            let session = s.sessions.get_mut(id).unwrap();
            for (k, v) in patch.as_object().unwrap() {
                session[k] = v.clone();
            }
        });
    }

    /// The donor pays: complete and paid.
    fn pay(&self, id: &str) {
        self.patch_session(
            id,
            json!({ "status": "complete", "payment_status": "paid" }),
        );
    }

    /// A session made by someone else, directly on Stripe (for forgeries).
    fn add_session(&self, id: &str, session: Value) {
        self.with(|s| s.sessions.insert(id.to_string(), session));
    }

    fn set_account(&self, id: &str, status: &str) {
        self.with(|s| {
            s.accounts
                .insert(id.to_string(), stripe_account(id, status))
        });
    }

    fn set_subscription(&self, id: &str, status: &str) {
        self.with(|s| {
            s.subscriptions
                .insert(id.to_string(), json!({ "id": id, "status": status }))
        });
    }
}

fn form_of(text: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(text.as_bytes())
        .into_owned()
        .collect()
}

fn stripe_answer(
    state: &Mutex<StripeState>,
    method: Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: String,
) -> Response {
    assert_eq!(headers["authorization"], "Bearer sk_test_slop");
    let path = uri.path().trim_start_matches('/').to_string();
    let form = if method == Method::GET {
        form_of(uri.query().unwrap_or(""))
    } else {
        form_of(&body)
    };
    let account = headers
        .get("stripe-account")
        .map(|v| v.to_str().unwrap().to_string());
    let mut s = state.lock().unwrap();
    s.calls.push(Call {
        method: method.to_string(),
        path: path.clone(),
        account: account.clone(),
        form: form.clone(),
        idempotency_key: headers
            .get("idempotency-key")
            .map(|v| v.to_str().unwrap().to_string()),
    });
    if let Some((.., status)) = s
        .failures
        .iter()
        .find(|(m, p, _)| *m == method.as_str() && path.starts_with(p.as_str()))
    {
        return (
            StatusCode::from_u16(*status).unwrap(),
            axum::Json(json!({ "error": { "message": "the fake Stripe says no" } })),
        )
            .into_response();
    }
    s.next += 1;
    let n = s.next;
    let parts: Vec<&str> = path.split('/').collect();
    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            axum::Json(json!({ "error": { "message": "No such object" } })),
        )
            .into_response()
    };
    let reply = match (method.as_str(), parts.as_slice()) {
        ("POST", ["v1", "accounts"]) => {
            let id = format!("acct_{n}");
            let status = s.new_account_status.unwrap_or("active");
            s.accounts.insert(id.clone(), stripe_account(&id, status));
            json!({ "id": id })
        }
        ("GET", ["v1", "accounts", id]) => s
            .accounts
            .get(*id)
            .cloned()
            .unwrap_or_else(|| stripe_account(id, "active")),
        ("POST", ["v1", "account_links"]) => {
            json!({ "url": format!("https://connect.stripe.example/setup/{}", form["account"]) })
        }
        ("POST", ["v1", "checkout", "sessions"]) => {
            let id = format!("cs_{n}");
            let metadata: serde_json::Map<String, Value> = form
                .iter()
                .filter_map(|(k, v)| {
                    k.strip_prefix("metadata[")
                        .map(|k| (k.trim_end_matches(']').to_string(), json!(v)))
                })
                .collect();
            let session = json!({
                "id": id, "status": "open", "payment_status": "unpaid",
                "mode": form.get("mode"),
                "url": format!("https://checkout.stripe.example/{id}"),
                "amount_total": form.iter()
                    .filter(|(k, _)| k.starts_with("line_items[") && k.ends_with("[price_data][unit_amount]"))
                    .map(|(_, v)| v.parse::<i64>().unwrap())
                    .sum::<i64>(),
                "currency": form.get("line_items[0][price_data][currency]").or(form.get("currency")),
                "application_fee": form.get("payment_intent_data[application_fee_amount]").map(|f| f.parse::<i64>().unwrap()),
                "metadata": metadata,
                "account": account,
                // What Slop sent, for tests to check.
                "form": form,
            });
            s.sessions.insert(id, session.clone());
            session
        }
        ("GET", ["v1", "checkout", "sessions", id]) => match s.sessions.get(*id) {
            // A session lives on one account: asked for on another, Stripe doesn't know it.
            Some(session) if session["account"].as_str() == account.as_deref() => {
                let mut session = session.clone();
                if session["payment_intent"].is_null() {
                    session["payment_intent"] = json!({
                        "latest_charge": format!("ch_{id}"),
                        "application_fee_amount": session["application_fee"]
                    });
                }
                session
            }
            _ => return not_found(),
        },
        ("POST", ["v1", "checkout", "sessions", id, "expire"]) => match s.sessions.get_mut(*id) {
            Some(session) => {
                session["status"] = json!("expired");
                session.clone()
            }
            None => return not_found(),
        },
        ("GET", ["v1", "subscriptions", id]) => s
            .subscriptions
            .get(*id)
            .cloned()
            .unwrap_or_else(|| json!({ "id": id, "status": "active" })),
        ("DELETE", ["v1", "subscriptions", id]) => {
            let sub = json!({ "id": id, "status": "canceled" });
            s.subscriptions.insert(id.to_string(), sub.clone());
            sub
        }
        ("POST", ["v1", "subscriptions", id]) => json!({ "id": id, "status": "active" }),
        ("GET", ["v1", "invoices", id]) => match s.invoices.get(*id) {
            Some(inv) => inv.clone(),
            None => return not_found(),
        },
        ("POST", ["v1", "refunds"]) => {
            json!({ "id": format!("re_{n}"), "charge": form.get("charge") })
        }
        ("GET", ["v1", "charges", id]) => s
            .charges
            .get(*id)
            .cloned()
            .unwrap_or_else(|| json!({ "id": id, "amount_refunded": 0 })),
        ("GET", ["v1", "application_fees"]) => {
            // Two to a page, as if `limit` were 2, so paging is exercised.
            let from = match form.get("starting_after") {
                Some(after) => {
                    s.application_fees
                        .iter()
                        .position(|f| f["id"] == after.as_str())
                        .unwrap()
                        + 1
                }
                None => 0,
            };
            let page: Vec<Value> = s
                .application_fees
                .iter()
                .skip(from)
                .take(2)
                .cloned()
                .collect();
            json!({ "data": page, "has_more": from + 2 < s.application_fees.len() })
        }
        ("POST", ["v1", "customers"]) => json!({ "id": format!("cus_{n}") }),
        ("POST", ["v1", "payment_methods"]) => json!({ "id": format!("pm_clone_{n}") }),
        ("POST", ["v1", "payment_methods", id, "detach"]) => json!({ "id": id }),
        ("POST", ["v1", "payment_intents"]) => {
            json!({ "id": format!("pi_{n}"), "status": s.intent_status.unwrap_or("succeeded") })
        }
        _ => return not_found(),
    };
    axum::Json(reply).into_response()
}

const WEBHOOK_SECRET: &str = "whsec_slop";

/// An app on the fake Stripe, with per-IP limits out of the way.
async fn stripe_app(db: PgPool, fake: &FakeStripe) -> TestApp {
    stripe_app_with(db, fake, |_| {}).await
}

async fn stripe_app_with(
    db: PgPool,
    fake: &FakeStripe,
    adjust: impl FnOnce(&mut Config),
) -> TestApp {
    let base = fake.base.clone();
    test_app_with(db, move |c| {
        c.payments.stripe = Some(slop_api::config::StripeConfig {
            secret_key: "sk_test_slop".into(),
            webhook_secret: WEBHOOK_SECRET.into(),
            api_base: base.parse().unwrap(),
        });
        c.rate_limits.write_per_minute = 1000;
        adjust(c);
    })
    .await
}

/// Simulated payments, with per-IP limits out of the way.
fn simulated(c: &mut Config) {
    c.payments.simulated = true;
    c.rate_limits.write_per_minute = 1000;
}

/// Emails go to the outbox.
fn with_email(c: &mut Config) {
    c.email = Some(slop_api::config::EmailConfig {
        smtp_url: "smtp://localhost:2525".into(),
        from: "n@slop.test".into(),
    });
}

/// A signed Stripe event, posted to the webhook. Returns the status Slop answered.
async fn hook(app: &TestApp, event: Value) -> StatusCode {
    let (body, sig) = stripe_signed(&event, WEBHOOK_SECRET);
    app.router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/stripe/webhook")
                .header(header::HOST, "localhost:8080")
                .header("stripe-signature", sig)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// A Stripe event of this type about `object`, on a connected account (or Slop's own).
fn event(id: &str, kind: &str, account: Option<&str>, object: Value) -> Value {
    json!({ "id": id, "type": kind, "account": account, "data": { "object": object } })
}

/// A user who has already confirmed they're 18 or older.
async fn adult(app: &TestApp, name: &str, country: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, country).await;
    sqlx::query("update users set adult_confirmed_at = now() where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    (id, cookie)
}

/// Gives a user an email address (the outbox only takes those).
async fn with_address(app: &TestApp, id: Uuid, email: &str) {
    sqlx::query("update users set email = $2 where id = $1")
        .bind(id)
        .bind(email)
        .execute(&app.db)
        .await
        .unwrap();
}

/// A maker whose app `slug` takes donations (and server donations with `servers`).
async fn maker_taking_donations(app: &TestApp, slug: &str, servers: bool) -> (Uuid, String) {
    let (maker_id, maker) = app.user("Mia Maker", "US").await;
    listed_with(app, maker_id, slug, "released", "").await;
    donations_on(app, &maker, slug, servers).await;
    (maker_id, maker)
}

/// A listed community build of `parent`, made by `owner`, with an approved release.
async fn community_build(app: &TestApp, parent: &str, slug: &str, owner: Uuid) {
    sqlx::query(
        "insert into apps (id, slug, title, summary, kind, stage, status, maker_id, repo_url, license_spdx,
                           parent_app_id, branch_kind, branch_note)
         select $1, $2, $2, 'A community build', 'game', 'released', 'listed', $3,
                'https://github.com/forker/' || $2, 'MIT', id, 'fix', 'Fixes things' from apps where slug = $4",
    )
    .bind(Uuid::now_v7())
    .bind(slug)
    .bind(owner)
    .bind(parent)
    .execute(&app.db)
    .await
    .unwrap();
    approved_release(app, slug, owner).await;
}

/// A staff member with one role.
async fn staff(app: &TestApp, name: &str, role: &str) -> (Uuid, String) {
    let (id, cookie) = app.user(name, "US").await;
    sqlx::query("update users set is_admin = true where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("insert into staff_roles (user_id, role, granted_by) values ($1, $2, 'test')")
        .bind(id)
        .bind(role)
        .execute(&app.db)
        .await
        .unwrap();
    (id, cookie)
}

/// A user who has a passkey and must use it now (their grace period is over), signed in
/// without it.
async fn passkey_holder(app: &TestApp, id: Uuid) {
    sqlx::query(
        "insert into passkeys (id, user_id, credential_id, credential, name) values ($1, $2, $3, '{}', 'Laptop')",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(id.as_bytes().to_vec())
    .execute(&app.db)
    .await
    .unwrap();
    sqlx::query(
        "update users set passkey_due_at = now() - interval '1 day', passkey_grace_until = null where id = $1",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
}

/// As if they had just confirmed with their passkey.
async fn passkey_confirmed(app: &TestApp, id: Uuid) {
    sqlx::query("update sessions set passkey_at = now() where user_id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
}

/// Makes the user's sign-in an hour old (too old for steps that need a fresh one).
async fn old_sign_in(app: &TestApp, id: Uuid) {
    sqlx::query(
        "update sessions set created_at = now() - interval '1 hour', stepped_up_at = null where user_id = $1",
    )
    .bind(id)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn notices(app: &TestApp, user: Uuid) -> Vec<String> {
    sqlx::query_scalar("select title from notifications where user_id = $1 order by created_at")
        .bind(user)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

async fn audit_actions(app: &TestApp, prefix: &str) -> Vec<String> {
    sqlx::query_scalar("select action from audit_log where action like $1 || '%' order by id")
        .bind(prefix)
        .fetch_all(&app.db)
        .await
        .unwrap()
}

/// Starts a donation; returns the status and body.
async fn donate(app: &TestApp, cookie: &str, slug: &str, body: Value) -> (StatusCode, Value) {
    let res = app
        .send(
            req(Method::POST, format!("/v1/apps/{slug}/donate"))
                .cookie(cookie)
                .json(body),
        )
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

async fn get_json(app: &TestApp, cookie: &str, uri: &str) -> Value {
    body_json(app.send(req(Method::GET, uri).cookie(cookie)).await).await
}

/// The proof-of-work answer to a fresh challenge, made quick to solve.
async fn solved_challenge(app: &TestApp, cookie: &str) -> Value {
    let challenge = body_json(
        app.send(req(Method::POST, "/v1/donations/challenge").cookie(cookie))
            .await,
    )
    .await;
    let id: Uuid = challenge["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("update donation_challenges set difficulty = 4 where id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    let nonce = slop_api::challenge::solve(challenge["prefix"].as_str().unwrap(), 4);
    json!({ "id": id, "nonce": nonce })
}
