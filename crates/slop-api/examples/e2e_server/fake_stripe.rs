// SPDX-License-Identifier: AGPL-3.0-or-later
//! A fake Stripe for the browser tests, modelled on the API tests' (tests/api/money_tests.rs):
//! accounts, Checkout sessions (payment, subscription and setup), subscriptions, invoices,
//! refunds, charges, application fees, customers, saved cards and payment intents.
//!
//! Unlike the API tests' it also stands in for Stripe's own pages, so the website can be driven
//! through a payment in a browser:
//!
//! - `GET /checkout/<session>`: a "checkout" page with **Pay** and **Cancel**. Paying completes
//!   the session as Stripe would, posts the signed webhooks to SLAPorSLOP (`checkout.session.completed`,
//!   and `invoice.paid` for a monthly one) and sends the browser to the session's `success_url`.
//! - `GET /setup/<account>`: Stripe's sign-up for a connected account; it makes the account
//!   active, tells SLAPorSLOP (`account.updated`) and sends the browser to the link's `return_url`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};

pub const SECRET_KEY: &str = "sk_test_e2e";
pub const WEBHOOK_SECRET: &str = "whsec_e2e";

#[derive(Default)]
struct StripeState {
    /// Checkout sessions by ID; `account` is the connected account each was made on.
    sessions: HashMap<String, Value>,
    accounts: HashMap<String, Value>,
    /// Where an account's sign-up link returns to.
    returns: HashMap<String, String>,
    subscriptions: HashMap<String, Value>,
    invoices: HashMap<String, Value>,
    application_fees: Vec<Value>,
    next: u32,
}

/// The fake Stripe on a local port. `base` ends in a slash.
#[derive(Clone)]
pub struct FakeStripe {
    pub base: String,
    state: Arc<Mutex<StripeState>>,
    /// SLAPorSLOP's webhook endpoint.
    webhook: String,
    http: reqwest::Client,
}

/// A Stripe account object: active, or pending (still signing up).
fn stripe_account(id: &str, active: bool) -> Value {
    if active {
        json!({ "id": id, "charges_enabled": true, "details_submitted": true, "requirements": { "disabled_reason": null } })
    } else {
        json!({ "id": id, "charges_enabled": false, "details_submitted": false, "requirements": { "disabled_reason": "requirements.pending_verification" } })
    }
}

fn form_of(text: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(text.as_bytes())
        .into_owned()
        .collect()
}

/// `metadata[k]` fields (after `prefix`) as an object.
fn metadata(form: &HashMap<String, String>, prefix: &str) -> serde_json::Map<String, Value> {
    form.iter()
        .filter_map(|(k, v)| {
            k.strip_prefix(prefix)
                .map(|k| (k.trim_end_matches(']').to_string(), json!(v)))
        })
        .collect()
}

impl FakeStripe {
    /// Starts it; `webhook` is SLAPorSLOP's `/v1/stripe/webhook`.
    pub async fn start(webhook: String) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let fake = FakeStripe {
            base: format!("http://{addr}/"),
            state: Arc::default(),
            webhook,
            http: reqwest::Client::new(),
        };
        let app = Router::new()
            .route("/checkout/{id}", get(checkout_page))
            .route("/checkout/{id}/pay", post(pay_page))
            .route("/setup/{account}", get(setup_page))
            .fallback(api)
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(fake)
    }

    fn with<T>(&self, f: impl FnOnce(&mut StripeState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// Makes a connected account active, as if its owner finished Stripe's sign-up, and tells
    /// SLAPorSLOP.
    pub async fn activate(&self, account: &str) -> anyhow::Result<()> {
        self.with(|s| {
            s.accounts
                .insert(account.to_string(), stripe_account(account, true))
        });
        self.send_event(
            "account.updated",
            Some(account),
            json!({ "id": account, "object": "account" }),
        )
        .await
    }

    /// Pays a Checkout session as its payer would, and tells SLAPorSLOP (the webhooks). Returns where
    /// Stripe would send the browser next.
    pub async fn pay(&self, id: &str) -> anyhow::Result<String> {
        let (session, invoice) = self.with(|s| {
            s.next += 1;
            let n = s.next;
            let session = s
                .sessions
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("no session {id}"))?;
            session["status"] = json!("complete");
            let mut invoice = None;
            match session["mode"].as_str() {
                Some("setup") => {
                    session["setup_intent"] =
                        json!({ "id": format!("seti_{n}"), "payment_method": format!("pm_card_{n}") });
                }
                Some("subscription") => {
                    session["payment_status"] = json!("paid");
                    let sub = format!("sub_{n}");
                    session["subscription"] = json!(sub);
                    let meta = session["subscription_metadata"].clone();
                    let inv = json!({
                        "id": format!("in_{n}"), "status": "paid", "subscription": sub,
                        "subscription_details": { "metadata": meta },
                        "amount_paid": session["amount_total"],
                        "status_transitions": { "paid_at": time::OffsetDateTime::now_utc().unix_timestamp() },
                    });
                    invoice = Some(inv.clone());
                    let account = session["account"].clone();
                    s.subscriptions.insert(
                        sub.clone(),
                        json!({ "id": sub, "status": "active", "account": account }),
                    );
                    s.invoices.insert(format!("in_{n}"), inv);
                }
                _ => {
                    session["payment_status"] = json!("paid");
                    session["payment_intent"] = json!({
                        "id": format!("pi_{n}"),
                        "latest_charge": format!("ch_{n}"),
                        "application_fee_amount": session["application_fee"],
                    });
                }
            }
            let session = session.clone();
            if let Some(fee) = session["application_fee"].as_i64() {
                s.application_fees.push(json!({
                    "id": format!("fee_{n}"), "amount": fee, "currency": session["currency"],
                    "account": session["account"], "created": time::OffsetDateTime::now_utc().unix_timestamp(),
                }));
            }
            anyhow::Ok((session, invoice))
        })?;
        let account = session["account"].as_str();
        self.send_event("checkout.session.completed", account, session.clone())
            .await?;
        if let Some(inv) = invoice {
            self.send_event("invoice.paid", account, inv).await?;
        }
        let success = session["form"]["success_url"].as_str().unwrap_or("/");
        Ok(success.replace("{CHECKOUT_SESSION_ID}", id))
    }

    /// Posts a signed event to SLAPorSLOP's webhook.
    async fn send_event(
        &self,
        kind: &str,
        account: Option<&str>,
        object: Value,
    ) -> anyhow::Result<()> {
        let n = self.with(|s| {
            s.next += 1;
            s.next
        });
        let body = json!({
            "id": format!("evt_e2e_{n}"), "type": kind, "account": account,
            "data": { "object": object },
        })
        .to_string();
        let t = time::OffsetDateTime::now_utc().unix_timestamp();
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes())?;
        mac.update(format!("{t}.{body}").as_bytes());
        let sig = format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()));
        let res = self
            .http
            .post(&self.webhook)
            .header("stripe-signature", sig)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        anyhow::ensure!(
            res.status().is_success(),
            "SLAPorSLOP answered the {kind} webhook with {}",
            res.status()
        );
        Ok(())
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Stripe's hosted checkout page, much simplified.
async fn checkout_page(State(fake): State<FakeStripe>, Path(id): Path<String>) -> Response {
    let Some(session) = fake.with(|s| s.sessions.get(&id).cloned()) else {
        return (StatusCode::NOT_FOUND, "No such checkout session").into_response();
    };
    let name = session["form"]["line_items[0][price_data][product_data][name]"]
        .as_str()
        .unwrap_or("Save your card");
    let amount = session["amount_total"].as_i64().unwrap_or(0);
    let currency = session["currency"].as_str().unwrap_or("usd").to_uppercase();
    let cancel = session["form"]["cancel_url"].as_str().unwrap_or("/");
    let what = match session["mode"].as_str() {
        Some("setup") => "Save a card for later".to_string(),
        Some("subscription") => format!("{} {currency} a month", amount as f64 / 100.0),
        _ => format!("{} {currency}", amount as f64 / 100.0),
    };
    let done = session["status"] == "complete";
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Fake Stripe Checkout</title></head>\
         <body><main><h1>Fake Stripe Checkout</h1><p>{}</p><p>{}</p>{}<p><a href=\"{}\">Cancel</a></p></main></body></html>",
        escape(name),
        escape(&what),
        if done {
            "<p>Already paid.</p>".to_string()
        } else {
            format!(
                "<form method=\"post\" action=\"/checkout/{}/pay\"><button type=\"submit\">Pay</button></form>",
                escape(&id)
            )
        },
        escape(cancel),
    ))
    .into_response()
}

async fn pay_page(State(fake): State<FakeStripe>, Path(id): Path<String>) -> Response {
    match fake.pay(&id).await {
        Ok(next) => Redirect::to(&next).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

/// Stripe's sign-up for a connected account: done at once.
async fn setup_page(State(fake): State<FakeStripe>, Path(account): Path<String>) -> Response {
    if let Err(err) = fake.activate(&account).await {
        return (StatusCode::BAD_GATEWAY, err.to_string()).into_response();
    }
    let back = fake
        .with(|s| s.returns.get(&account).cloned())
        .unwrap_or_else(|| "/".into());
    Redirect::to(&back).into_response()
}

/// Stripe's API.
async fn api(
    State(fake): State<FakeStripe>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some(&format!("Bearer {SECRET_KEY}"))
    {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": { "message": "Invalid API key" } })),
        )
            .into_response();
    }
    let path = uri.path().trim_start_matches('/').to_string();
    let form = if method == Method::GET {
        form_of(uri.query().unwrap_or(""))
    } else {
        form_of(&body)
    };
    let account = headers
        .get("stripe-account")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let base = fake.base.clone();
    let mut s = fake.state.lock().unwrap();
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
        // New accounts are still signing up until their owner finishes Stripe's pages.
        ("POST", ["v1", "accounts"]) => {
            let id = format!("acct_e2e_{n}");
            s.accounts.insert(id.clone(), stripe_account(&id, false));
            json!({ "id": id })
        }
        ("GET", ["v1", "accounts", id]) => s
            .accounts
            .get(*id)
            .cloned()
            .unwrap_or_else(|| stripe_account(id, true)),
        ("POST", ["v1", "account_links"]) => {
            let acct = form.get("account").cloned().unwrap_or_default();
            if let Some(back) = form.get("return_url") {
                s.returns.insert(acct.clone(), back.clone());
            }
            json!({ "url": format!("{base}setup/{acct}") })
        }
        ("POST", ["v1", "checkout", "sessions"]) => {
            let id = format!("cs_e2e_{n}");
            let session = json!({
                "id": id, "object": "checkout.session", "status": "open", "payment_status": "unpaid",
                "mode": form.get("mode"),
                "url": format!("{base}checkout/{id}"),
                "amount_total": form.iter()
                    .filter(|(k, _)| k.starts_with("line_items[") && k.ends_with("[price_data][unit_amount]"))
                    .map(|(_, v)| v.parse::<i64>().unwrap_or(0))
                    .sum::<i64>(),
                "currency": form.get("line_items[0][price_data][currency]").or(form.get("currency")),
                "customer": form.get("customer"),
                "application_fee": form.get("payment_intent_data[application_fee_amount]").and_then(|f| f.parse::<i64>().ok()),
                "metadata": metadata(&form, "metadata["),
                "subscription_metadata": metadata(&form, "subscription_data[metadata]["),
                "account": account,
                "form": form,
            });
            s.sessions.insert(id, session.clone());
            session
        }
        ("GET", ["v1", "checkout", "sessions", id]) => match s.sessions.get(*id) {
            // A session lives on one account: asked for on another, Stripe doesn't know it.
            Some(session) if session["account"].as_str() == account.as_deref() => session.clone(),
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
        ("GET", ["v1", "invoice_payments"]) => {
            json!({ "object": "list", "data": [], "has_more": false })
        }
        ("GET", ["v1", "charges", id]) => json!({ "id": id, "amount_refunded": 0 }),
        ("GET", ["v1", "application_fees"]) => {
            let from = form
                .get("starting_after")
                .and_then(|after| {
                    s.application_fees
                        .iter()
                        .position(|f| f["id"] == after.as_str())
                })
                .map_or(0, |i| i + 1);
            let page: Vec<Value> = s
                .application_fees
                .iter()
                .skip(from)
                .take(100)
                .cloned()
                .collect();
            json!({ "data": page, "has_more": from + 100 < s.application_fees.len() })
        }
        ("POST", ["v1", "customers"]) => json!({ "id": format!("cus_{n}") }),
        ("DELETE", ["v1", "customers", id]) => json!({ "id": id, "deleted": true }),
        ("POST", ["v1", "payment_methods"]) => json!({ "id": format!("pm_clone_{n}") }),
        ("POST", ["v1", "payment_methods", id, "detach"]) => json!({ "id": id }),
        ("POST", ["v1", "payment_intents"]) => {
            json!({ "id": format!("pi_{n}"), "status": "succeeded", "latest_charge": format!("ch_{n}") })
        }
        _ => return not_found(),
    };
    axum::Json(reply).into_response()
}
