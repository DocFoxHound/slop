//! Stripe Connect for People payments (PAYMENTS §3, §7, §8).
//!
//! - Each creative connects **their own** Stripe account (full Stripe dashboard, Stripe carries
//!   losses: what Stripe used to call "Standard"). Slop stores only the account ID and status.
//! - Each job payment is a **direct charge** on that account through Stripe Checkout, with
//!   `application_fee_amount` = Slop's 5%. Slop never holds the money.
//! - Webhooks are checked against Stripe's signature, handled once per event ID, and the state
//!   is always re-read from Stripe's API rather than trusted from the webhook body.
//!
//! Without Stripe keys, payments are **simulated** on the simulation sandbox and on development
//! servers that ask for it (`SLOP_PAYMENTS=simulated`); otherwise they're off.

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{jobs, my_creative};
use crate::{
    AppState,
    auth::session::CurrentUser,
    config::StripeConfig,
    error::{ApiError, ApiResult},
};

/// Slop's platform fee, in basis points (PAYMENTS §2). VAT, where due, comes out of it.
pub const FEE_BASIS_POINTS: i64 = 500;

pub fn fee(amount_minor: i64) -> i64 {
    (amount_minor * FEE_BASIS_POINTS + 5_000) / 10_000
}

pub enum Mode<'a> {
    Stripe(&'a StripeConfig),
    Simulated,
    Off,
}

impl Mode<'_> {
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Stripe(_) => "stripe",
            Mode::Simulated => "simulated",
            Mode::Off => "off",
        }
    }
}

pub fn mode(state: &AppState) -> Mode<'_> {
    match (
        &state.config.payments.stripe,
        state.config.payments.simulated,
    ) {
        (Some(stripe), _) => Mode::Stripe(stripe),
        (None, true) => Mode::Simulated,
        (None, false) => Mode::Off,
    }
}

pub fn webhook_routes() -> Router<AppState> {
    Router::new().route("/stripe/webhook", post(webhook))
}

// ---------------------------------------------------------------------------------------------
// Stripe's API (form-encoded requests, JSON answers)

pub(crate) async fn call(
    state: &AppState,
    stripe: &StripeConfig,
    method: reqwest::Method,
    path: &str,
    account: Option<&str>,
    form: &[(&str, String)],
) -> anyhow::Result<Value> {
    let mut url = stripe.api_base.join(path)?;
    if method == reqwest::Method::GET && !form.is_empty() {
        url.query_pairs_mut()
            .extend_pairs(form.iter().map(|(k, v)| (*k, v.as_str())));
    }
    let mut req = state
        .http
        .request(method.clone(), url)
        .bearer_auth(&stripe.secret_key);
    if let Some(acct) = account {
        req = req.header("stripe-account", acct);
    }
    if method != reqwest::Method::GET {
        req = req.form(form);
    }
    let res = req.send().await?;
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let message = body["error"]["message"].as_str().unwrap_or("no message");
        anyhow::bail!("Stripe answered {status}: {message}");
    }
    Ok(body)
}

pub(crate) fn account_status(account: &Value) -> &'static str {
    if account["charges_enabled"].as_bool() == Some(true)
        && account["details_submitted"].as_bool() == Some(true)
    {
        "active"
    } else if account["requirements"]["disabled_reason"]
        .as_str()
        .is_some_and(|r| !r.is_empty() && !r.starts_with("requirements.pending"))
        && account["details_submitted"].as_bool() == Some(true)
    {
        "restricted"
    } else {
        "pending"
    }
}

// ---------------------------------------------------------------------------------------------
// Onboarding (PAYMENTS §4)

#[derive(Serialize)]
pub struct Onboarding {
    /// Stripe's sign-up page to send the creative to; none when simulated.
    url: Option<String>,
    status: String,
}

pub async fn onboard(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Onboarding>> {
    let me = my_creative(&state, &user).await?;
    crate::passkeys::require(&state, &user).await?;
    let existing: Option<String> =
        sqlx::query_scalar("select stripe_account_id from creatives where user_id = $1")
            .bind(me)
            .fetch_one(&state.db)
            .await?;
    match mode(&state) {
        Mode::Off => Err(ApiError::Unavailable(
            "payments aren't switched on yet on this server",
        )),
        Mode::Simulated => {
            sqlx::query(
                "update creatives set stripe_account_id = coalesce(stripe_account_id, $2),
                        stripe_status = 'active', updated_at = now() where user_id = $1",
            )
            .bind(me)
            .bind(format!("acct_sim_{}", Uuid::new_v4().simple()))
            .execute(&state.db)
            .await?;
            Ok(Json(Onboarding {
                url: None,
                status: "active".into(),
            }))
        }
        Mode::Stripe(stripe) => {
            let account = match existing {
                Some(a) => a,
                None => {
                    let created = call(
                        &state,
                        stripe,
                        reqwest::Method::POST,
                        "v1/accounts",
                        None,
                        &[
                            ("controller[stripe_dashboard][type]", "full".into()),
                            ("controller[fees][payer]", "account".into()),
                            ("controller[losses][payments]", "stripe".into()),
                            ("controller[requirement_collection]", "stripe".into()),
                            ("metadata[slop_user]", me.to_string()),
                        ],
                    )
                    .await?;
                    let id = created["id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Stripe gave no account ID"))?
                        .to_string();
                    sqlx::query(
                        "update creatives set stripe_account_id = $2, stripe_status = 'pending', updated_at = now()
                         where user_id = $1",
                    )
                    .bind(me)
                    .bind(&id)
                    .execute(&state.db)
                    .await?;
                    id
                }
            };
            let base = &state.config.public_url;
            let link = call(
                &state,
                stripe,
                reqwest::Method::POST,
                "v1/account_links",
                None,
                &[
                    ("account", account),
                    ("type", "account_onboarding".into()),
                    (
                        "refresh_url",
                        base.join("people/me?stripe=again")
                            .map_err(anyhow::Error::from)?
                            .to_string(),
                    ),
                    (
                        "return_url",
                        base.join("people/me?stripe=back")
                            .map_err(anyhow::Error::from)?
                            .to_string(),
                    ),
                ],
            )
            .await?;
            Ok(Json(Onboarding {
                url: link["url"].as_str().map(String::from),
                status: "pending".into(),
            }))
        }
    }
}

/// Re-reads the creative's Stripe account (after they come back from Stripe's sign-up).
pub async fn refresh_account(
    State(state): State<AppState>,
    user: CurrentUser,
) -> ApiResult<Json<Onboarding>> {
    let me = my_creative(&state, &user).await?;
    let (account, status): (Option<String>, String) =
        sqlx::query_as("select stripe_account_id, stripe_status from creatives where user_id = $1")
            .bind(me)
            .fetch_one(&state.db)
            .await?;
    let status = match (mode(&state), account) {
        (Mode::Stripe(stripe), Some(acct)) => sync_account(&state, stripe, &acct).await?,
        _ => status,
    };
    Ok(Json(Onboarding { url: None, status }))
}

pub(crate) async fn sync_account(
    state: &AppState,
    stripe: &StripeConfig,
    acct: &str,
) -> anyhow::Result<String> {
    let account = call(
        state,
        stripe,
        reqwest::Method::GET,
        &format!("v1/accounts/{acct}"),
        None,
        &[],
    )
    .await?;
    let status = account_status(&account);
    // One Stripe account can take People payments and donations.
    for table in ["creatives", "donation_accounts"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update {table} set stripe_status = $2, updated_at = now() where stripe_account_id = $1"
        )))
        .bind(acct)
        .bind(status)
        .execute(&state.db)
        .await?;
    }
    // A donations account waiting to be switched to (donations.rs).
    sqlx::query(
        "update stripe_account_changes set new_account_status = $2 where new_account_id = $1",
    )
    .bind(acct)
    .bind(status)
    .execute(&state.db)
    .await?;
    Ok(status.to_string())
}

// ---------------------------------------------------------------------------------------------
// Paying (jobs.rs calls these)

pub struct Due {
    pub job_payment_id: Uuid,
    pub job_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub description: String,
    pub account: String,
    pub session: Option<String>,
}

/// Where to send the client to pay: Stripe Checkout, reusing an open session so a second
/// press can't charge twice.
pub async fn checkout_url(
    state: &AppState,
    stripe: &StripeConfig,
    due: &Due,
    payer: Uuid,
) -> anyhow::Result<String> {
    if let Some(session) = &due.session {
        let s = call(
            state,
            stripe,
            reqwest::Method::GET,
            &format!("v1/checkout/sessions/{session}"),
            Some(&due.account),
            &[],
        )
        .await?;
        if s["status"] == "open"
            && let Some(url) = s["url"].as_str()
        {
            return Ok(url.to_string());
        }
        if s["status"] == "complete" {
            sync_session(state, stripe, &due.account, session).await?;
            anyhow::bail!("this payment has already been made");
        }
    }
    let base = &state.config.public_url;
    let fee = fee(due.amount_minor);
    let job = due.job_id;
    let session = call(
        state,
        stripe,
        reqwest::Method::POST,
        "v1/checkout/sessions",
        Some(&due.account),
        &[
            ("mode", "payment".into()),
            ("client_reference_id", due.job_payment_id.to_string()),
            ("metadata[job_payment_id]", due.job_payment_id.to_string()),
            ("metadata[payer]", payer.to_string()),
            ("line_items[0][quantity]", "1".into()),
            ("line_items[0][price_data][currency]", due.currency.clone()),
            (
                "line_items[0][price_data][unit_amount]",
                due.amount_minor.to_string(),
            ),
            (
                "line_items[0][price_data][product_data][name]",
                due.description.clone(),
            ),
            (
                "payment_intent_data[application_fee_amount]",
                fee.to_string(),
            ),
            ("payment_intent_data[description]", due.description.clone()),
            (
                "payment_intent_data[metadata][job_payment_id]",
                due.job_payment_id.to_string(),
            ),
            (
                "success_url",
                format!("{}jobs/{job}?paid={{CHECKOUT_SESSION_ID}}", base),
            ),
            ("cancel_url", format!("{}jobs/{job}", base)),
        ],
    )
    .await?;
    let id = session["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no session"))?;
    sqlx::query("update job_payments set checkout_session_id = $2 where id = $1")
        .bind(due.job_payment_id)
        .bind(id)
        .execute(&state.db)
        .await?;
    session["url"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("Stripe gave no checkout link"))
}

/// Reads a Checkout session from Stripe and, when it's paid, records the payment.
pub async fn sync_session(
    state: &AppState,
    stripe: &StripeConfig,
    account: &str,
    session_id: &str,
) -> anyhow::Result<bool> {
    let s = call(
        state,
        stripe,
        reqwest::Method::GET,
        &format!("v1/checkout/sessions/{session_id}"),
        Some(account),
        &[("expand[]", "payment_intent".into())],
    )
    .await?;
    if s["payment_status"] != "paid" {
        return Ok(false);
    }
    let job_payment_id: Uuid = s["metadata"]["job_payment_id"]
        .as_str()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("session without a job payment"))?;
    let payer: Option<Uuid> = s["metadata"]["payer"].as_str().and_then(|v| v.parse().ok());
    let charge = s["payment_intent"]["latest_charge"]
        .as_str()
        .unwrap_or(session_id)
        .to_string();
    let amount = s["amount_total"].as_i64().unwrap_or_default();
    let fee = s["payment_intent"]["application_fee_amount"]
        .as_i64()
        .unwrap_or_else(|| fee(amount));
    jobs::payment_succeeded(
        &state.db,
        jobs::Paid {
            job_payment_id,
            stripe_account_id: Some(account.to_string()),
            object_id: charge,
            payer,
            amount_minor: amount,
            fee_minor: fee,
            simulated: false,
        },
    )
    .await?;
    Ok(true)
}

// ---------------------------------------------------------------------------------------------
// Webhooks

/// Checks `Stripe-Signature: t=…,v1=…` (HMAC-SHA256 of "t.body"), within 5 minutes.
pub fn signature_ok(secret: &str, body: &[u8], header: Option<&str>, now: i64) -> bool {
    let Some(header) = header else {
        return false;
    };
    let mut t = None;
    let mut sigs = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse::<i64>().ok(),
            Some(("v1", v)) => sigs.extend(hex::decode(v).ok()),
            _ => {}
        }
    }
    let Some(t) = t else {
        return false;
    };
    if (now - t).abs() > 300 {
        return false;
    }
    sigs.iter().any(|sig| {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
            return false;
        };
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        mac.verify_slice(sig).is_ok()
    })
}

#[derive(Deserialize)]
struct Event {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    /// The connected account the event happened on.
    account: Option<String>,
    data: EventData,
}

#[derive(Deserialize)]
struct EventData {
    object: Value,
}

async fn webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Mode::Stripe(stripe) = mode(&state) else {
        return StatusCode::NOT_FOUND;
    };
    let sig = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok());
    let now = OffsetDateTime::now_utc().unix_timestamp();
    if !signature_ok(&stripe.webhook_secret, &body, sig, now) {
        return StatusCode::BAD_REQUEST;
    }
    let Ok(event) = serde_json::from_slice::<Event>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    match handle(&state, stripe, &event).await {
        Ok(()) => StatusCode::OK,
        Err(err) => {
            tracing::error!(event = %event.id, kind = %event.kind, error = ?err, "stripe webhook failed");
            // Stripe retries; the event isn't marked handled.
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn handle(state: &AppState, stripe: &StripeConfig, event: &Event) -> anyhow::Result<()> {
    let seen: bool =
        sqlx::query_scalar("select exists (select 1 from stripe_events where id = $1)")
            .bind(&event.id)
            .fetch_one(&state.db)
            .await?;
    if seen {
        return Ok(());
    }
    let object_id = event.data.object["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    match (event.kind.as_str(), event.account.as_deref()) {
        ("checkout.session.completed" | "checkout.session.async_payment_succeeded", Some(acct)) => {
            // Which kind of payment it is decides who reads it; the state comes from Stripe's API.
            let meta = &event.data.object["metadata"];
            if meta["donation_id"].is_string() || meta["server_donation_id"].is_string() {
                crate::donations::sync_checkout(state, stripe, acct, &object_id).await?;
            } else {
                sync_session(state, stripe, acct, &object_id).await?;
            }
        }
        ("invoice.paid", Some(acct)) => {
            crate::donations::sync_invoice(state, stripe, acct, &object_id).await?;
        }
        ("customer.subscription.deleted", Some(acct)) => {
            crate::donations::subscription_ended(state, stripe, acct, &object_id).await?;
        }
        ("charge.refunded", Some(acct)) => {
            let charge = call(
                state,
                stripe,
                reqwest::Method::GET,
                &format!("v1/charges/{object_id}"),
                Some(acct),
                &[],
            )
            .await?;
            let refunded = charge["amount_refunded"].as_i64().unwrap_or_default();
            jobs::refund_recorded(&state.db, &object_id, refunded).await?;
            crate::donations::refund_recorded(&state.db, &object_id, refunded).await?;
        }
        ("charge.dispute.created", Some(_)) => {
            let charge = event.data.object["charge"].as_str().unwrap_or_default();
            let amount = event.data.object["amount"].as_i64().unwrap_or_default();
            jobs::dispute_recorded(&state.db, charge, &object_id, amount).await?;
        }
        ("account.updated", _) => {
            sync_account(state, stripe, &object_id).await?;
        }
        _ => {}
    }
    sqlx::query("insert into stripe_events (id, kind) values ($1, $2) on conflict do nothing")
        .bind(&event.id)
        .bind(&event.kind)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// For the job page: what a payment will look like to both sides.
pub fn breakdown(amount_minor: i64) -> Value {
    json!({ "pay": amount_minor, "slop_fee": fee(amount_minor) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fee_is_five_percent_rounded() {
        assert_eq!(fee(10_000), 500);
        assert_eq!(fee(1_010), 51);
        assert_eq!(fee(1_009), 50);
    }

    #[test]
    fn signatures_are_checked_with_a_time_limit() {
        let body = br#"{"id":"evt_1"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"whsec_test").unwrap();
        mac.update(b"1000.");
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        let header = format!("t=1000,v1=deadbeef,v1={sig}");
        assert!(signature_ok("whsec_test", body, Some(&header), 1100));
        assert!(!signature_ok("whsec_test", body, Some(&header), 2000));
        assert!(!signature_ok("other", body, Some(&header), 1100));
        assert!(!signature_ok("whsec_test", b"{}", Some(&header), 1100));
        assert!(!signature_ok("whsec_test", body, None, 1100));
    }
}
