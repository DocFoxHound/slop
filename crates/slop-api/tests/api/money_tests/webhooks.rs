// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stripe's webhook: signatures, handling each event once, and each kind of event (donation
//! checkouts, monthly invoices, ended subscriptions, refunds, disputes, accounts, saved cards for
//! pledges), plus People job checkouts.

use super::*;

/// Posts a raw body to the webhook, signed with the right secret.
async fn hook_raw(app: &TestApp, body: &str) -> StatusCode {
    use hmac::{Hmac, KeyInit, Mac};
    let t = OffsetDateTime::now_utc().unix_timestamp();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    let sig = format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()));
    app.router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/stripe/webhook")
                .header(header::HOST, "localhost:8080")
                .header("stripe-signature", sig)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// A maker on Stripe taking donations (with servers) and their account.
async fn stripe_maker(app: &TestApp) -> (Uuid, String, String) {
    let (maker_id, maker) = maker_taking_donations(app, "arena", true).await;
    let acct: String = sqlx::query_scalar("select stripe_account_id from donation_accounts")
        .fetch_one(&app.db)
        .await
        .unwrap();
    (maker_id, maker, acct)
}

/// Starts a donation on Stripe: (its ID, its Checkout session).
async fn started(app: &TestApp, cookie: &str, body: Value) -> (String, String) {
    let (status, started) = donate(app, cookie, "arena", body).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let session = started["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    (started["id"].as_str().unwrap().to_string(), session)
}

fn completed(id: &str, account: &str, session: &str, meta: Value) -> Value {
    event(
        id,
        "checkout.session.completed",
        Some(account),
        json!({ "id": session, "metadata": meta }),
    )
}

async fn donation_status(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar(
        "select status from donations where id = $1 union all select status from server_donations where id = $1",
    )
    .bind(id.parse::<Uuid>().unwrap())
    .fetch_one(&app.db)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn without_stripe_there_is_no_webhook(db: PgPool) {
    let app = test_app_with(db, simulated).await;
    let status = hook(&app, event("evt_1", "customer.created", None, json!({}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn the_webhook_handles_each_event_once_and_lets_stripe_retry_failures(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;

    // Signed but not an event.
    assert_eq!(hook_raw(&app, "not json").await, StatusCode::BAD_REQUEST);
    assert_eq!(hook_raw(&app, "{}").await, StatusCode::BAD_REQUEST);
    // Kinds SLAPorSLOP doesn't use are recorded and acknowledged.
    let status = hook(
        &app,
        event(
            "evt_other",
            "customer.created",
            None,
            json!({ "id": "cus_1" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let kind: String = sqlx::query_scalar("select kind from stripe_events where id = 'evt_other'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(kind, "customer.created");

    // Stripe can't be read: 500, not recorded, so Stripe's retry is handled.
    let (id, session) = started(&app, &dee, json!({ "amount_minor": 300 })).await;
    fake.pay(&session);
    let paid = completed("evt_paid", &acct, &session, json!({ "donation_id": id }));
    fake.fail("GET", "v1/checkout/sessions", 503);
    assert_eq!(
        hook(&app, paid.clone()).await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from stripe_events where id = 'evt_paid'"
        )
        .await,
        0
    );
    assert_eq!(donation_status(&app, &id).await, "pending");
    fake.heal();
    assert_eq!(hook(&app, paid.clone()).await, StatusCode::OK);
    assert_eq!(donation_status(&app, &id).await, "paid");
    let reads = fake.calls("GET", "v1/checkout/sessions").len();
    // Delivered again: acknowledged without being handled again.
    assert_eq!(hook(&app, paid).await, StatusCode::OK);
    assert_eq!(fake.calls("GET", "v1/checkout/sessions").len(), reads);
    assert_eq!(
        count(
            &app,
            "select count(*) from stripe_events where id = 'evt_paid'"
        )
        .await,
        1
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn donation_checkout_webhooks_trust_only_the_session_slop_made(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, _, acct) = stripe_maker(&app).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;

    // Someone's own Stripe account makes a paid session naming Dee's donation: ignored.
    let (id, session) = started(&app, &dee, json!({ "amount_minor": 300 })).await;
    fake.add_session(
        "cs_evil",
        json!({ "id": "cs_evil", "status": "complete", "payment_status": "paid", "amount_total": 340,
                "currency": "usd", "account": "acct_evil", "metadata": { "donation_id": id } }),
    );
    let status = hook(
        &app,
        completed(
            "evt_evil",
            "acct_evil",
            "cs_evil",
            json!({ "donation_id": id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(donation_status(&app, &id).await, "pending");
    // The real session, for a different amount or currency: ignored.
    fake.pay(&session);
    fake.patch_session(&session, json!({ "amount_total": 300 }));
    hook(
        &app,
        completed("evt_cheap", &acct, &session, json!({ "donation_id": id })),
    )
    .await;
    assert_eq!(donation_status(&app, &id).await, "pending");
    fake.patch_session(&session, json!({ "amount_total": 340, "currency": "eur" }));
    hook(
        &app,
        completed("evt_eur", &acct, &session, json!({ "donation_id": id })),
    )
    .await;
    assert_eq!(donation_status(&app, &id).await, "pending");
    assert_eq!(count(&app, "select count(*) from payments").await, 0);

    // As made: paid once, thanked once, however many events say so.
    fake.patch_session(&session, json!({ "currency": "usd" }));
    for (evt, kind) in [
        ("evt_ok", "checkout.session.completed"),
        ("evt_ok2", "checkout.session.async_payment_succeeded"),
    ] {
        let status = hook(
            &app,
            event(
                evt,
                kind,
                Some(&acct),
                json!({ "id": session, "metadata": { "donation_id": id } }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(donation_status(&app, &id).await, "paid");
    assert_eq!(count(&app, "select count(*) from payments").await, 1);
    let thanked = notices(&app, dee_id).await;
    assert_eq!(
        thanked
            .iter()
            .filter(|t| *t == "Thank you for donating to arena")
            .count(),
        1
    );
    assert_eq!(
        notices(&app, maker_id)
            .await
            .iter()
            .filter(|t| *t == "New donations to arena")
            .count(),
        1
    );

    // A monthly one: the subscription starts, with Stripe's ID.
    let (monthly, monthly_session) = started(
        &app,
        &eve,
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    fake.patch_session(
        &monthly_session,
        json!({ "status": "complete", "subscription": "sub_eve" }),
    );
    let status = hook(
        &app,
        completed(
            "evt_monthly",
            &acct,
            &monthly_session,
            json!({ "server_donation_id": monthly }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(donation_status(&app, &monthly).await, "active");
    let sub: Option<String> =
        sqlx::query_scalar("select stripe_subscription_id from server_donations")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(sub.as_deref(), Some("sub_eve"));
}

/// New: Stripe says when a Checkout page expires unpaid; the donation then stops showing as
/// waiting (before, only the donor coming back to SLAPorSLOP marked it).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn an_expired_checkout_marks_its_donation_expired(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (id, session) = started(&app, &dee, json!({ "amount_minor": 300 })).await;
    fake.patch_session(&session, json!({ "status": "expired" }));
    let status = hook(
        &app,
        event(
            "evt_expired",
            "checkout.session.expired",
            Some(&acct),
            json!({ "id": session, "metadata": { "donation_id": id } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(donation_status(&app, &id).await, "expired");
    assert!(
        get_json(&app, &dee, "/v1/me/donations").await["donations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn invoice_webhooks_record_each_month_once_for_subscriptions_slop_started(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (_, eve) = adult(&app, "Eve Donor", "US").await;
    let (id, session) = started(
        &app,
        &dee,
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    fake.patch_session(
        &session,
        json!({ "status": "complete", "subscription": "sub_dee" }),
    );
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    let months = || {
        count(
            &app,
            "select count(*) from donations where kind = 'monthly'",
        )
    };
    let paid_at = (OffsetDateTime::now_utc() + time::Duration::days(30)).unix_timestamp();
    let invoice = |inv: &str, body: Value| {
        let mut body = body;
        body["id"] = json!(inv);
        fake.with(|s| s.invoices.insert(inv.to_string(), body));
    };
    let paid = |evt: &str, inv: &str, account: &str| {
        event(evt, "invoice.paid", Some(account), json!({ "id": inv }))
    };

    // A month paid: its own donation and payment, once per invoice.
    invoice(
        "in_1",
        json!({ "status": "paid", "subscription": "sub_dee",
                "subscription_details": { "metadata": { "server_donation_id": id } },
                "status_transitions": { "paid_at": paid_at } }),
    );
    assert_eq!(
        hook(&app, paid("evt_1", "in_1", &acct)).await,
        StatusCode::OK
    );
    assert_eq!(
        hook(&app, paid("evt_1b", "in_1", &acct)).await,
        StatusCode::OK
    );
    assert_eq!(months().await, 1);
    let (object, amount): (String, i32) = sqlx::query_as(
        "select stripe_object_id, amount_minor from payments where stripe_object_id = 'in_1'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((object.as_str(), amount), ("in_1", 546));
    let next: OffsetDateTime =
        sqlx::query_scalar("select next_charge_at from server_donations where id = $1")
            .bind(id.parse::<Uuid>().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        next.unix_timestamp(),
        paid_at + time::Duration::days(30).whole_seconds(),
        "a month after this payment"
    );

    // Not paid, or another subscription naming Dee's donation, or another account: ignored.
    invoice(
        "in_open",
        json!({ "status": "open", "subscription": "sub_dee",
                "subscription_details": { "metadata": { "server_donation_id": id } } }),
    );
    invoice(
        "in_evil",
        json!({ "status": "paid", "subscription": "sub_evil",
                "subscription_details": { "metadata": { "server_donation_id": id } } }),
    );
    assert_eq!(
        hook(&app, paid("evt_2", "in_open", &acct)).await,
        StatusCode::OK
    );
    assert_eq!(
        hook(&app, paid("evt_3", "in_evil", &acct)).await,
        StatusCode::OK
    );
    assert_eq!(
        hook(&app, paid("evt_4", "in_evil", "acct_evil")).await,
        StatusCode::OK
    );
    assert_eq!(months().await, 1);

    // Stripe's newer shape, with the subscription under `parent`.
    invoice(
        "in_2",
        json!({ "status": "paid",
                "parent": { "subscription_details": { "subscription": "sub_dee",
                                                      "metadata": { "server_donation_id": id } } } }),
    );
    assert_eq!(
        hook(&app, paid("evt_5", "in_2", &acct)).await,
        StatusCode::OK
    );
    assert_eq!(months().await, 2);

    // The first invoice can arrive before the session's own event: Stripe is asked which
    // subscription the session started.
    let (eve_id, eve_session) = started(
        &app,
        &eve,
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    fake.patch_session(&eve_session, json!({ "subscription": "sub_eve" }));
    invoice(
        "in_eve",
        json!({ "status": "paid", "subscription": "sub_eve",
                "subscription_details": { "metadata": { "server_donation_id": eve_id } } }),
    );
    invoice(
        "in_eve_evil",
        json!({ "status": "paid", "subscription": "sub_other",
                "subscription_details": { "metadata": { "server_donation_id": eve_id } } }),
    );
    assert_eq!(
        hook(&app, paid("evt_6", "in_eve_evil", &acct)).await,
        StatusCode::OK
    );
    assert_eq!(donation_status(&app, &eve_id).await, "pending");
    assert_eq!(
        hook(&app, paid("evt_7", "in_eve", &acct)).await,
        StatusCode::OK
    );
    assert_eq!(donation_status(&app, &eve_id).await, "active");
    let sub: Option<String> =
        sqlx::query_scalar("select stripe_subscription_id from server_donations where id = $1")
            .bind(eve_id.parse::<Uuid>().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(sub.as_deref(), Some("sub_eve"));
    assert_eq!(months().await, 3);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn subscription_webhooks_cancel_only_what_stripe_cancelled(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (id, session) = started(
        &app,
        &dee,
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    fake.patch_session(
        &session,
        json!({ "status": "complete", "subscription": "sub_dee" }),
    );
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    // The event's body is never trusted: Stripe is asked.
    for (n, status) in ["active", "past_due"].into_iter().enumerate() {
        fake.set_subscription("sub_dee", status);
        let res = hook(
            &app,
            event(
                &format!("evt_{n}"),
                "customer.subscription.deleted",
                Some(&acct),
                json!({ "id": "sub_dee", "status": "canceled" }),
            ),
        )
        .await;
        assert_eq!(res, StatusCode::OK);
        assert_eq!(donation_status(&app, &id).await, "active", "{status}");
    }
    fake.set_subscription("sub_dee", "canceled");
    hook(
        &app,
        event(
            "evt_gone",
            "customer.subscription.deleted",
            Some(&acct),
            json!({ "id": "sub_dee" }),
        ),
    )
    .await;
    let (status, by): (String, String) =
        sqlx::query_as("select status, cancelled_by from server_donations")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((status.as_str(), by.as_str()), ("cancelled", "stripe"));
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn refund_webhooks_record_partial_and_full_refunds_once(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (id, session) = started(&app, &dee, json!({ "amount_minor": 300 })).await;
    fake.pay(&session);
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    let charge = format!("ch_{session}");
    let refunded = |evt: &str, total: i64| {
        let charge = charge.clone();
        fake.with(|s| {
            s.charges.insert(
                charge.clone(),
                json!({ "id": charge, "amount_refunded": total }),
            )
        });
        event(evt, "charge.refunded", Some(&acct), json!({ "id": charge }))
    };
    let state = || {
        let app = &app;
        let id = id.clone();
        async move {
            sqlx::query_as::<_, (String, i32, i64)>(
                "select status, refunded_minor,
                        (select coalesce(sum(amount_minor), 0)::int8 from payments where kind = 'refund' and donation_id = d.id)
                 from donations d where id = $1",
            )
            .bind(id.parse::<Uuid>().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap()
        }
    };
    assert_eq!(hook(&app, refunded("evt_r1", 100)).await, StatusCode::OK);
    assert_eq!(state().await, ("partially_refunded".into(), 100, -100));
    assert_eq!(hook(&app, refunded("evt_r1b", 100)).await, StatusCode::OK);
    assert_eq!(
        state().await,
        ("partially_refunded".into(), 100, -100),
        "once"
    );
    // The rest: the record holds the total refunded, not the two added up twice.
    assert_eq!(hook(&app, refunded("evt_r2", 340)).await, StatusCode::OK);
    assert_eq!(state().await, ("refunded".into(), 340, -340));
    let original: Uuid = sqlx::query_scalar("select id from payments where kind = 'donation'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(
        count(
            &app,
            &format!(
                "select count(*) from payments where kind = 'refund' and refund_of <> '{original}'"
            )
        )
        .await,
        0
    );
    // A charge SLAPorSLOP doesn't know: nothing.
    let other = event(
        "evt_r3",
        "charge.refunded",
        Some(&acct),
        json!({ "id": "ch_unknown" }),
    );
    assert_eq!(hook(&app, other).await, StatusCode::OK);
    assert_eq!(
        count(&app, "select count(*) from payments where kind = 'refund'").await,
        2
    );
}

/// b40: refunds and disputes on bounty payouts and donations are money moving back; the payments
/// record (append-only, PAYMENTS §8) should show them as it does for People jobs.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn disputes_on_donations_and_bounty_refunds_are_recorded(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    let (dee_id, dee) = adult(&app, "Dee Donor", "US").await;
    let (id, session) = started(&app, &dee, json!({ "amount_minor": 300 })).await;
    fake.pay(&session);
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    let status = hook(
        &app,
        event(
            "evt_dispute",
            "charge.dispute.created",
            Some(&acct),
            json!({ "id": "dp_1", "charge": format!("ch_{session}"), "amount": 340 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        count(&app, "select count(*) from payments where kind = 'dispute'").await,
        1,
        "a donation's dispute"
    );
    // A bounty payout's charge, refunded from the builder's dashboard.
    sqlx::query(
        "insert into payments (id, kind, stripe_account_id, stripe_object_id, payer_id, amount_minor, slop_fee_minor, currency)
         values ($1, 'bounty', $2, 'pi_bounty', $3, 2000, 100, 'usd')",
    )
    .bind(Uuid::now_v7())
    .bind(&acct)
    .bind(dee_id)
    .execute(&app.db)
    .await
    .unwrap();
    fake.with(|s| {
        s.charges.insert(
            "ch_bounty".into(),
            json!({ "id": "ch_bounty", "payment_intent": "pi_bounty", "amount_refunded": 2000 }),
        )
    });
    let status = hook(
        &app,
        event(
            "evt_bounty_refund",
            "charge.refunded",
            Some(&acct),
            json!({ "id": "ch_bounty", "payment_intent": "pi_bounty" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        count(
            &app,
            "select coalesce(sum(amount_minor), 0)::int8 from payments where kind = 'refund'
             and refund_of = (select id from payments where stripe_object_id = 'pi_bounty')"
        )
        .await,
        -2000
    );
    // A dispute on the same charge, told twice: recorded once, against the pledge's payment.
    for evt in ["evt_bounty_dispute", "evt_bounty_dispute_again"] {
        let status = hook(
            &app,
            event(
                evt,
                "charge.dispute.created",
                Some(&acct),
                json!({ "id": "dp_2", "charge": "ch_bounty", "amount": 2000 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(
        count(
            &app,
            "select count(*) from payments where kind = 'dispute'
             and refund_of = (select id from payments where stripe_object_id = 'pi_bounty')"
        )
        .await,
        1
    );
}

/// On Stripe's API versions from 2025-03-31 a charge no longer names its invoice, so a monthly
/// donation's refund and dispute are matched through Stripe's invoice payments.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn monthly_refunds_and_disputes_match_on_newer_stripe_versions(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    let (_, dee) = adult(&app, "Dee Donor", "US").await;
    let (id, session) = started(
        &app,
        &dee,
        json!({ "amount_minor": 500, "monthly": true, "consent": true }),
    )
    .await;
    fake.patch_session(
        &session,
        json!({ "status": "complete", "subscription": "sub_dee" }),
    );
    app.send(req(Method::POST, format!("/v1/me/donations/{id}/check")).cookie(&dee))
        .await;
    fake.with(|s| {
        s.invoices.insert(
            "in_1".into(),
            json!({ "id": "in_1", "status": "paid",
                    "parent": { "subscription_details": { "subscription": "sub_dee",
                                                          "metadata": { "server_donation_id": id } } } }),
        );
        // The newer shape: no `invoice` field at all.
        s.charges.insert(
            "ch_month".into(),
            json!({ "id": "ch_month", "payment_intent": "pi_month", "amount_refunded": 546 }),
        );
        s.invoice_payments.insert("pi_month".into(), "in_1".into());
    });
    assert_eq!(
        hook(
            &app,
            event(
                "evt_paid",
                "invoice.paid",
                Some(&acct),
                json!({ "id": "in_1" })
            )
        )
        .await,
        StatusCode::OK
    );
    let against_the_month = |kind: &'static str| {
        count(
            &app,
            match kind {
                "refund" => "select coalesce(sum(amount_minor), 0)::int8 from payments where kind = 'refund'
                             and refund_of = (select id from payments where stripe_object_id = 'in_1')",
                _ => "select count(*) from payments where kind = 'dispute'
                      and refund_of = (select id from payments where stripe_object_id = 'in_1')",
            },
        )
    };
    let status = hook(
        &app,
        event(
            "evt_refund",
            "charge.refunded",
            Some(&acct),
            json!({ "id": "ch_month", "payment_intent": "pi_month" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(against_the_month("refund").await, -546);
    let status = hook(
        &app,
        event(
            "evt_dispute",
            "charge.dispute.created",
            Some(&acct),
            json!({ "id": "dp_month", "charge": "ch_month", "amount": 546 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(against_the_month("dispute").await, 1);
    let looked_up = fake.calls("GET", "v1/invoice_payments");
    assert!(
        looked_up
            .iter()
            .all(|c| c.account.as_deref() == Some(acct.as_str())
                && c.form["payment[payment_intent]"] == "pi_month"),
        "asked on the maker's account, by payment intent"
    );
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn account_webhooks_update_every_place_the_account_is_used(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (vera_id, vera) = app.user("Vera", "US").await;
    let mut p = profile("vera", false);
    p["confirm_adult"] = json!(true);
    app.send(req(Method::PUT, "/v1/me/creative").cookie(&vera).json(p))
        .await;
    app.send(req(Method::POST, "/v1/me/creative/stripe").cookie(&vera))
        .await;
    app.send(
        req(Method::POST, "/v1/me/donation-account")
            .cookie(&vera)
            .json(json!({ "tax_notice_ack": true })),
    )
    .await;
    let acct: String =
        sqlx::query_scalar("select stripe_account_id from creatives where user_id = $1")
            .bind(vera_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let statuses = || {
        let app = &app;
        async move {
            sqlx::query_as::<_, (String, String)>(
                "select (select stripe_status from creatives), (select stripe_status from donation_accounts)",
            )
            .fetch_one(&app.db)
            .await
            .unwrap()
        }
    };
    for (n, status) in ["restricted", "pending", "active"].into_iter().enumerate() {
        fake.set_account(&acct, status);
        // The event says nothing SLAPorSLOP uses but the account's ID.
        let res = hook(
            &app,
            event(
                &format!("evt_acct_{n}"),
                "account.updated",
                Some(&acct),
                json!({ "id": acct, "charges_enabled": true, "details_submitted": true }),
            ),
        )
        .await;
        assert_eq!(res, StatusCode::OK);
        assert_eq!(statuses().await, (status.to_string(), status.to_string()));
    }
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn pledge_webhooks_activate_a_saved_card_and_never_revive_a_withdrawn_pledge(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let (bob_id, bob) = adult(&app, "Bob Backer", "US").await;
    let bounty = body_json(
        app.send(
            req(Method::POST, "/v1/apps/arena/bounties")
                .cookie(&ann)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "usd" })),
        )
        .await,
    )
    .await;
    let id = bounty["id"].as_str().unwrap();
    let pledge = |cookie: &str| {
        req(Method::POST, format!("/v1/bounties/{id}/pledge"))
            .cookie(cookie)
            .json(json!({ "amount_minor": 1000 }))
    };
    let session_of = |answer: Value| {
        answer["url"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap()
            .to_string()
    };
    let ann_session = session_of(body_json(app.send(pledge(&ann)).await).await);
    let bob_session = session_of(body_json(app.send(pledge(&bob)).await).await);
    let pledge_status = |user: Uuid| {
        let app = &app;
        async move {
            sqlx::query_as::<_, (String, Option<String>)>(
                "select status, payment_method_id from bounty_pledges where user_id = $1",
            )
            .bind(user)
            .fetch_one(&app.db)
            .await
            .unwrap()
        }
    };
    let saved = |evt: &str, session: &str| {
        event(
            evt,
            "checkout.session.completed",
            None,
            json!({ "id": session, "metadata": { "bounty_pledge_id": "x" } }),
        )
    };
    // Not finished: still saving.
    assert_eq!(
        hook(&app, saved("evt_1", &ann_session)).await,
        StatusCode::OK
    );
    assert_eq!(pledge_status(ann_id).await, ("saving".into(), None));
    fake.patch_session(
        &ann_session,
        json!({ "status": "complete", "setup_intent": { "payment_method": "pm_ann" } }),
    );
    assert_eq!(
        hook(&app, saved("evt_2", &ann_session)).await,
        StatusCode::OK
    );
    assert_eq!(
        pledge_status(ann_id).await,
        ("active".into(), Some("pm_ann".into()))
    );
    // Bob withdraws before his card is saved; Stripe finishing it afterwards changes nothing.
    assert_eq!(
        app.send(req(Method::DELETE, format!("/v1/bounties/{id}/pledge")).cookie(&bob))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    fake.patch_session(
        &bob_session,
        json!({ "status": "complete", "setup_intent": { "payment_method": "pm_bob" } }),
    );
    assert_eq!(
        hook(&app, saved("evt_3", &bob_session)).await,
        StatusCode::OK
    );
    assert_eq!(pledge_status(bob_id).await, ("withdrawn".into(), None));
}

/// b37: closing an account withdraws its pledges and deletes its Stripe customer, so no card
/// stays saved for it.
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn closing_an_account_removes_its_saved_cards(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (maker_id, _) = app.user("Mia Maker", "US").await;
    listed_with(&app, maker_id, "arena", "released", "").await;
    let (ann_id, ann) = adult(&app, "Ann Backer", "US").await;
    let bounty = body_json(
        app.send(
            req(Method::POST, "/v1/apps/arena/bounties")
                .cookie(&ann)
                .json(json!({ "title": "Fix saves", "body": "They vanish.", "currency": "usd" })),
        )
        .await,
    )
    .await;
    let id = bounty["id"].as_str().unwrap();
    let started = body_json(
        app.send(
            req(Method::POST, format!("/v1/bounties/{id}/pledge"))
                .cookie(&ann)
                .json(json!({ "amount_minor": 1000 })),
        )
        .await,
    )
    .await;
    let session = started["url"].as_str().unwrap().rsplit('/').next().unwrap();
    fake.patch_session(
        session,
        json!({ "status": "complete", "setup_intent": { "payment_method": "pm_ann" } }),
    );
    let saved = event(
        "evt_saved",
        "checkout.session.completed",
        None,
        json!({ "id": session, "metadata": { "bounty_pledge_id": "x" } }),
    );
    assert_eq!(hook(&app, saved).await, StatusCode::OK);
    let customer: String =
        sqlx::query_scalar("select customer_id from stripe_customers where user_id = $1")
            .bind(ann_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let res = app
        .send(
            req(Method::POST, "/v1/me/close")
                .cookie(&ann)
                .json(json!({ "confirm": "close my account" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        fake.calls("DELETE", &format!("v1/customers/{customer}"))
            .len(),
        1
    );
    assert_eq!(
        count(&app, "select count(*) from stripe_customers").await,
        0
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from bounty_pledges where status = 'withdrawn'"
        )
        .await,
        1
    );
}

/// A People job the client has accepted, on a creative with a Stripe account: (job ID, the
/// creative's account, the client's cookie).
async fn job_to_pay(app: &TestApp) -> (String, String, String) {
    let (vera_id, vera) = live_creative(app, "vera", "US", false).await;
    app.send(req(Method::POST, "/v1/me/creative/stripe/refresh").cookie(&vera))
        .await;
    let acct: String =
        sqlx::query_scalar("select stripe_account_id from creatives where user_id = $1")
            .bind(vera_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let (client, client_cookie) = app.user("Client", "US").await;
    listed_with(app, client, "tavern", "beta", "").await;
    let job = body_json(
        app.send(
            req(Method::POST, "/v1/people/vera/hire")
                .cookie(&client_cookie)
                .json(
                    json!({ "app_slug": "tavern", "skill": "pixel-art", "request_md": "Knight",
                              "confirm_adult": true, "exact_wording_ack": true }),
                ),
        )
        .await,
    )
    .await;
    let id = job["id"].as_str().unwrap().to_string();
    app.send(
        req(Method::POST, format!("/v1/jobs/{id}/offer"))
            .cookie(&vera)
            .json(sprite_offer("upfront")),
    )
    .await;
    let res = app
        .send(
            req(Method::POST, format!("/v1/jobs/{id}/accept"))
                .cookie(&client_cookie)
                .json(json!({ "offer_count": 1, "exact_terms_ack": true })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::OK, "{:?}", body_json(res).await);
    (id, acct, client_cookie)
}

async fn pay_job(app: &TestApp, id: &str, client: &str) -> (StatusCode, Value) {
    let res = app
        .send(req(Method::POST, format!("/v1/jobs/{id}/pay")).cookie(client))
        .await;
    let status = res.status();
    (status, body_json(res).await)
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn job_checkouts_are_made_again_when_expired_and_never_paid_twice(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (id, acct, client) = job_to_pay(&app).await;
    let (status, first) = pay_job(&app, &id, &client).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let session = first["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(fake.session(&session)["account"], acct.as_str());

    // Expired unpaid: a new Checkout page.
    fake.patch_session(&session, json!({ "status": "expired" }));
    let (_, again) = pay_job(&app, &id, &client).await;
    let second = again["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    assert_ne!(second, session);
    let stored: Option<String> = sqlx::query_scalar("select checkout_session_id from job_payments")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored.as_deref(), Some(second.as_str()));

    // A paid session that isn't the one made for this payment (any more) is ignored.
    fake.pay(&session);
    let status = hook(
        &app,
        completed("evt_old", &acct, &session, json!({ "job_payment_id": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count(&app, "select count(*) from payments").await, 0);
    fake.patch_session(&second, json!({ "amount_total": 100 }));
    fake.pay(&second);
    let job = body_json(
        app.send(req(Method::POST, format!("/v1/jobs/{id}/payment/check")).cookie(&client))
            .await,
    )
    .await;
    assert_eq!(job["status"], "awaiting_payment", "{job}");
    assert_eq!(count(&app, "select count(*) from payments").await, 0);

    // Paid as made: pressing "Pay" again records it and says so, without a new Checkout page.
    fake.patch_session(&second, json!({ "amount_total": 12001 }));
    let made = fake.calls("POST", "v1/checkout/sessions").len();
    let (status, body) = pay_job(&app, &id, &client).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already been made"),
        "{body}"
    );
    assert_eq!(fake.calls("POST", "v1/checkout/sessions").len(), made);
    assert_eq!(count(&app, "select count(*) from payments").await, 1);
    let job = get_json(&app, &client, &format!("/v1/jobs/{id}")).await;
    assert_eq!(job["status"], "in_progress");
}

/// b29: Stripe sends every event on a connected account, including the maker's own sales that
/// have nothing to do with SLAPorSLOP. Those are acknowledged, not failed (Stripe would retry them for
/// days and then switch the endpoint off).
#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn a_makers_own_stripe_sales_are_acknowledged_and_ignored(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (_, _, acct) = stripe_maker(&app).await;
    fake.add_session(
        "cs_shop",
        json!({ "id": "cs_shop", "status": "complete", "payment_status": "paid", "amount_total": 2500,
                "currency": "usd", "account": acct, "metadata": {} }),
    );
    let status = hook(
        &app,
        event(
            "evt_shop",
            "checkout.session.completed",
            Some(&acct),
            json!({ "id": "cs_shop", "metadata": {} }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        count(
            &app,
            "select count(*) from stripe_events where id = 'evt_shop'"
        )
        .await,
        1
    );
    assert_eq!(count(&app, "select count(*) from payments").await, 0);
    // One naming a job payment SLAPorSLOP never made: the same.
    fake.add_session(
        "cs_odd",
        json!({ "id": "cs_odd", "status": "complete", "payment_status": "paid", "amount_total": 2500,
                "currency": "usd", "account": acct, "metadata": { "job_payment_id": Uuid::now_v7() } }),
    );
    let status = hook(
        &app,
        event(
            "evt_odd",
            "checkout.session.completed",
            Some(&acct),
            json!({ "id": "cs_odd", "metadata": {} }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "slop_api::MIGRATOR")]
async fn job_disputes_are_recorded_once(db: PgPool) {
    let fake = FakeStripe::start().await;
    let app = stripe_app(db, &fake).await;
    let (id, acct, client) = job_to_pay(&app).await;
    let (_, first) = pay_job(&app, &id, &client).await;
    let session = first["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    fake.pay(&session);
    hook(
        &app,
        completed(
            "evt_paid",
            &acct,
            &session,
            json!({ "job_payment_id": "x" }),
        ),
    )
    .await;
    assert_eq!(
        count(
            &app,
            "select count(*) from payments where kind = 'people_job'"
        )
        .await,
        1
    );
    let dispute = |evt: &str| {
        event(
            evt,
            "charge.dispute.created",
            Some(&acct),
            json!({ "id": "dp_1", "charge": format!("ch_{session}"), "amount": 12001 }),
        )
    };
    assert_eq!(hook(&app, dispute("evt_d1")).await, StatusCode::OK);
    assert_eq!(hook(&app, dispute("evt_d2")).await, StatusCode::OK);
    let (amount, refund_of_kind): (i32, String) = sqlx::query_as(
        "select d.amount_minor, o.kind from payments d join payments o on o.id = d.refund_of where d.kind = 'dispute'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((amount, refund_of_kind.as_str()), (-12001, "people_job"));
    let job = get_json(&app, &client, &format!("/v1/jobs/{id}")).await;
    let disputed = job["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "disputed")
        .count();
    assert_eq!(disputed, 1, "{job}");
}
