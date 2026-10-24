// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the browser tests start with: an account for every kind of person, and something in
//! every area of the site. Made through the API's own routes where there is one (called
//! in-process, as each account), and with SQL only where there's no route. web/e2e/fixtures.ts
//! lists it all for the tests.

use std::collections::BTreeMap;

use anyhow::{Context, bail};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use slop_api::{auth::session, config::Config};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::fake_stripe::FakeStripe;

pub struct Ctx {
    pub db: PgPool,
    pub config: Config,
    pub router: Router,
    /// The fake GitHub's base URL (ends in a slash).
    pub fake: String,
    pub stripe: FakeStripe,
}

/// Sign-in cookies by account, and the IDs and slugs tests need.
pub struct Seeded {
    pub cookies: BTreeMap<&'static str, String>,
    pub ids: BTreeMap<&'static str, Value>,
}

/// A seeded account.
#[derive(Clone)]
pub struct Who {
    pub id: Uuid,
    /// The session cookie's value.
    pub cookie: String,
}

impl Ctx {
    /// Calls the API as `who` (or signed out). Fails unless it answers 2xx.
    pub async fn call(
        &self,
        who: Option<&Who>,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> anyhow::Result<Value> {
        let (status, value) = self.try_call(who, method.clone(), uri, body).await?;
        if !status.is_success() {
            bail!("{method} {uri} answered {status}: {value}");
        }
        Ok(value)
    }

    /// Calls the API; returns the status and the JSON answer (null when it's not JSON).
    pub async fn try_call(
        &self,
        who: Option<&Who>,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> anyhow::Result<(axum::http::StatusCode, Value)> {
        let host = self
            .config
            .public_url
            .host_str()
            .context("public URL host")?
            .to_string();
        let host = match self.config.public_url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host,
        };
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, host)
            .header(header::USER_AGENT, "e2e seed");
        if let Some(who) = who {
            builder = builder.header(
                header::COOKIE,
                format!("{}={}", session::cookie_name(&self.config), who.cookie),
            );
        }
        let body = match body {
            Some(json) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let res = self.router.clone().oneshot(builder.body(body)?).await?;
        let status = res.status();
        let bytes = res.into_body().collect().await?.to_bytes();
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }

    pub async fn get(&self, who: Option<&Who>, uri: &str) -> anyhow::Result<Value> {
        self.call(who, Method::GET, uri, None).await
    }

    pub async fn post(&self, who: &Who, uri: &str, body: Value) -> anyhow::Result<Value> {
        self.call(Some(who), Method::POST, uri, Some(body)).await
    }

    pub async fn put(&self, who: &Who, uri: &str, body: Value) -> anyhow::Result<Value> {
        self.call(Some(who), Method::PUT, uri, Some(body)).await
    }

    /// Runs one statement.
    pub async fn sql(&self, sql: &str, binds: &[&dyn ToBind]) -> anyhow::Result<()> {
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.to_string()));
        for b in binds {
            q = b.bind_to(q);
        }
        q.execute(&self.db)
            .await
            .with_context(|| format!("SQL: {sql}"))?;
        Ok(())
    }

    /// A signed-in account (as the API tests make them: agreements accepted, 16 or older).
    pub async fn user(&self, name: &str, country: &str) -> anyhow::Result<Who> {
        let id = Uuid::now_v7();
        self.sql(
            "insert into users (id, display_name, country_code, age_16_confirmed_at) values ($1, $2, $3, now())",
            &[&id, &name, &country],
        )
        .await?;
        self.sql(
            "insert into legal_acceptances (user_id, kind, version)
             select $1, kind, version from legal_documents where kind in ('terms', 'publisher', 'people')",
            &[&id],
        )
        .await?;
        let cookie = session::create(&self.db, &self.config, id, Some("e2e")).await?;
        Ok(Who {
            id,
            cookie: cookie.value().to_string(),
        })
    }

    /// An account that has said it's 18 or older (donating, pledging, hiring).
    pub async fn adult(&self, name: &str, country: &str) -> anyhow::Result<Who> {
        let who = self.user(name, country).await?;
        self.sql(
            "update users set adult_confirmed_at = now() where id = $1",
            &[&who.id],
        )
        .await?;
        Ok(who)
    }

    /// Staff with these roles.
    pub async fn staff(&self, name: &str, roles: &[&str]) -> anyhow::Result<Who> {
        let who = self.adult(name, "US").await?;
        self.sql("update users set is_admin = true where id = $1", &[&who.id])
            .await?;
        for role in roles {
            self.sql(
                "insert into staff_roles (user_id, role, granted_by) values ($1, $2, 'e2e seed')",
                &[&who.id, role],
            )
            .await?;
        }
        Ok(who)
    }

    pub async fn app_id(&self, slug: &str) -> anyhow::Result<Uuid> {
        sqlx::query_scalar("select id from apps where slug = $1")
            .bind(slug)
            .fetch_one(&self.db)
            .await
            .with_context(|| format!("app {slug}"))
    }
}

/// Something `Ctx::sql` can bind.
pub trait ToBind: Sync {
    fn bind_to<'q>(
        &'q self,
        q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>;
}

macro_rules! to_bind {
    ($($t:ty),*) => {$(
        impl ToBind for $t {
            fn bind_to<'q>(
                &'q self,
                q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
            ) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
                q.bind(self)
            }
        }
    )*};
}
to_bind!(
    Uuid,
    &str,
    String,
    i32,
    i64,
    bool,
    Option<Uuid>,
    Option<String>
);

pub async fn seed(ctx: &Ctx) -> anyhow::Result<Seeded> {
    let mut s = Seeded {
        cookies: BTreeMap::new(),
        ids: BTreeMap::new(),
    };
    // Kept as they were: the first browser tests rely on them.
    let maker = ctx.user("Mia Maker", "US").await?;
    let player = ctx.user("Pat Player", "US").await?;
    let admin = ctx.staff("Ada Admin", &["owner"]).await?;
    let sam = ctx.user("Sam Seed", "US").await?;
    // A publisher past their passkey deadline, with no passkey (passkeys.rs).
    let overdue = ctx.user("Olly Overdue", "US").await?;
    ctx.sql(
        "update users set passkey_due_at = now() - interval '1 day' where id = $1",
        &[&overdue.id],
    )
    .await?;
    listed(ctx, sam.id, "castle-rock", "Castle Rock", "").await?;
    listed(
        ctx,
        sam.id,
        "neon-drift",
        "Neon Drift",
        "online = '{player_servers}'",
    )
    .await?;
    listed(ctx, maker.id, "mias-first", "Mia's First Game", "").await?;
    // Messages Mia's AI agent wrote to the admins (admin_messages.rs), for the AI messages tab.
    agent_message(
        ctx,
        maker.id,
        "report",
        "Release upload times out",
        "Submitting 1.2.0 timed out after 30 s.\nLog: https://example.com/log <b>not bold</b>",
    )
    .await?;
    agent_message(
        ctx,
        maker.id,
        "suggestion",
        "Show scan progress",
        "A percentage while files are scanned would help.",
    )
    .await?;
    for (name, who) in [
        ("maker", &maker),
        ("player", &player),
        ("admin", &admin),
        ("overdue", &overdue),
    ] {
        s.cookies.insert(name, who.cookie.clone());
    }
    s.ids.insert("maker", json!(maker.id));
    s.ids.insert("player", json!(player.id));
    s.ids.insert("admin", json!(admin.id));
    s.ids.insert("sam", json!(sam.id));
    s.ids.insert("overdue", json!(overdue.id));

    // More people: a teen (the under-18 view comes from the country: GB), a second maker, staff
    // with one role each, and players who don't sign in (their votes make the ratings show).
    let teen = ctx.user("Tia Teen", "GB").await?;
    let maker2 = ctx.user("Bo Builder", "US").await?;
    let mut fans = Vec::new();
    for name in ["Kai Kestrel", "Lu Lark", "Noor Nightjar"] {
        fans.push(ctx.user(name, "US").await?);
    }
    let mut staff = Vec::new();
    for (key, name, role) in [
        ("moderator", "Mo Moderator", "moderator"),
        ("reviewer", "Rae Reviewer", "reviewer"),
        ("support", "Sue Support", "support"),
        ("finance", "Fin Finance", "finance"),
        ("security", "Sec Security", "security"),
    ] {
        let who = ctx.staff(name, &[role]).await?;
        s.cookies.insert(key, who.cookie.clone());
        s.ids.insert(key, json!(who.id));
        staff.push(who);
    }
    s.cookies.insert("teen", teen.cookie.clone());
    s.cookies.insert("maker2", maker2.cookie.clone());
    s.ids.insert("teen", json!(teen.id));
    s.ids.insert("maker2", json!(maker2.id));

    // Pia Picture: a linked GitHub sign-in, whose picture the fake GitHub serves, for the profile
    // picture tests (accounts-picture.e2e.ts). Nobody else has a linked sign-in.
    let pictured = ctx.user("Pia Picture", "US").await?;
    ctx.sql(
        "insert into identities (provider, provider_subject, user_id, login) values ('github', '5150', $1, 'pia-gh')",
        &[&pictured.id],
    )
    .await?;
    s.cookies.insert("pictured", pictured.cookie.clone());
    s.ids.insert("pictured", json!(pictured.id));

    // Nia Newcomer: accepted only the Terms of Service, as at sign-up: no Publisher Agreement or
    // People Terms yet (AI agent tokens that publish, and the legal page's Accept).
    let newcomer = ctx.user("Nia Newcomer", "US").await?;
    ctx.sql(
        "delete from legal_acceptances where user_id = $1 and kind <> 'terms'",
        &[&newcomer.id],
    )
    .await?;
    s.cookies.insert("newcomer", newcomer.cookie.clone());
    s.ids.insert("newcomer", json!(newcomer.id));

    let p = people(ctx, &admin).await.context("People")?;
    s.cookies.insert("creative", p.creative.cookie.clone());
    s.cookies.insert("client", p.client.cookie.clone());
    s.cookies
        .insert("pendingCreative", p.pending_creative.cookie.clone());
    s.ids.insert("creative", json!(p.creative.id));
    s.ids.insert("creativeHandle", json!("vera-pixels"));
    s.ids.insert("client", json!(p.client.id));
    s.ids.insert("clientApp", json!("cleos-tavern"));
    s.ids
        .insert("pendingCreative", json!(p.pending_creative.id));
    s.ids.insert("pendingCreativeHandle", json!("remy-sounds"));
    s.ids.insert("deliveredJob", json!(p.delivered_job));
    s.ids.insert("workingJob", json!(p.working_job));
    s.ids.insert("requestedJob", json!(p.requested_job));
    s.ids.insert("problemJob", json!(p.problem_job));
    s.ids.insert("completedJob", json!(p.completed_job));

    store(
        ctx,
        &StoreCast {
            admin: &admin,
            maker: &maker,
            maker2: &maker2,
            player: &player,
            client: &p.client,
            teen: &teen,
            fans: &fans,
        },
        &mut s.ids,
    )
    .await
    .context("store")?;

    social(
        ctx,
        &SocialCast {
            player: &player,
            maker2: &maker2,
            client: &p.client,
            teen: &teen,
            maker: &maker,
            creative: &p.creative,
            admin: &admin,
        },
        &mut s.ids,
    )
    .await
    .context("social")?;

    // Suspended by staff (it can still sign in, read, and appeal).
    let suspended = ctx.user("Sid Suspended", "US").await?;
    s.cookies.insert("suspended", suspended.cookie.clone());
    s.ids.insert("suspended", json!(suspended.id));
    admin_queues(
        ctx,
        &AdminCast {
            admin: &admin,
            maker2: &maker2,
            client: &p.client,
            fan: &fans[2],
            suspended: &suspended,
            sam: sam.id,
        },
        &mut s.ids,
    )
    .await
    .context("admin queues")?;
    // Publishing through the API started Mia's 30 days to add a passkey (passkeys.rs). The
    // browser tests check her /publish page for accessibility as it was before (no deadline
    // yet), so it's cleared again; the other makers keep theirs. (The deadline's notice fails
    // axe's link-in-text-block check: web/src/components/PasskeyNotice.tsx.)
    ctx.sql(
        "update users set passkey_due_at = null where id = $1",
        &[&maker.id],
    )
    .await?;
    verify(ctx).await?;
    Ok(s)
}

/// A listed game by `maker`, with an approved release.
async fn listed(
    ctx: &Ctx,
    maker: Uuid,
    slug: &str,
    title: &str,
    extra: &str,
) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    ctx.sql(
        "insert into apps (id, slug, title, summary, description_md, kind, stage, status, maker_id, repo_url,
                           license_spdx, platforms, last_commit_at)
         values ($1, $2, $3, $4, $5, 'game', 'released', 'listed', $6, $7, 'MIT', '{windows}', now())",
        &[
            &id,
            &slug,
            &title,
            &format!("{title}: a game for the browser tests."),
            &format!("# {title}\n\nMade for Slop's browser tests."),
            &maker,
            &format!("https://github.com/e2e/{slug}"),
        ],
    )
    .await?;
    if !extra.is_empty() {
        ctx.sql(&format!("update apps set {extra} where id = $1"), &[&id])
            .await?;
    }
    ctx.sql(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, reviewed_at, commit_verified)
         values ($1, $2, '1.0.0', repeat('a', 40), 'approved', $3, now(), true)",
        &[&Uuid::now_v7(), &id, &maker],
    )
    .await?;
    Ok(id)
}

/// A thread an account's AI agent started with every admin, with a token made 12 days before.
async fn agent_message(
    ctx: &Ctx,
    sender: Uuid,
    category: &str,
    subject: &str,
    body: &str,
) -> anyhow::Result<()> {
    let id = Uuid::now_v7();
    ctx.sql(
        "insert into admin_threads (id, sender_id, via, category, subject) values ($1, $2, 'maker_agent', $3, $4)",
        &[&id, &sender, &category, &subject],
    )
    .await?;
    ctx.sql(
        "insert into admin_thread_messages (id, thread_id, author_id, from_staff, by_agent, body,
                token_kind, token_id, token_name, token_scope, token_created_at)
         values ($1, $2, $3, false, true, $4, 'maker', $5, 'Claude Code', 'read', now() - interval '12 days 1 hour')",
        &[&Uuid::now_v7(), &id, &sender, &body, &Uuid::now_v7()],
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// People

/// A made-up picture, as the People simulation makes them (sim_people.rs).
fn svg_picture(path: &str, w: u32, h: u32, color: &str, mark: &str) -> Value {
    use base64::Engine as _;
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' viewBox='0 0 {w} {h}'>\
         <rect width='100%' height='100%' fill='{color}'/>\
         <text x='50%' y='52%' font-size='{}' text-anchor='middle' dominant-baseline='middle' fill='#ffffffcc' font-family='sans-serif'>{mark}</text></svg>",
        h / 3,
    );
    let url = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    );
    json!({
        "path": path, "sha256": format!("{:064x}", url.len()), "url": url,
        "width": w, "height": h, "format": "svg",
    })
}

/// A portfolio waiting for review, as if checked from the creative's repository. Returns its ID.
async fn portfolio(ctx: &Ctx, creative: Uuid, handle: &str, color: &str) -> anyhow::Result<Uuid> {
    let works: Vec<Value> = ["Knight", "Tavern", "Dragon"]
        .iter()
        .map(|t| {
            let mut w = svg_picture(
                &format!("works/{}.svg", t.to_lowercase()),
                640,
                480,
                color,
                t,
            );
            w["kind"] = json!("image");
            w["title"] = json!(t);
            w["caption"] = json!(format!("{t}, 32×32, four directions"));
            w["skill"] = json!("pixel-art");
            w
        })
        .collect();
    let set = Uuid::now_v7();
    sqlx::query(
        "insert into portfolio_sets (id, creative_id, repo_url, commit_sha, avatar, banner, works, fingerprint, status, source)
         values ($1, $2, $3, repeat('c', 40), $4, $5, $6, $7, 'pending', 'creative')",
    )
    .bind(set)
    .bind(creative)
    .bind(format!("https://github.com/e2e/{handle}-portfolio"))
    .bind(sqlx::types::Json(svg_picture("avatar.svg", 256, 256, color, "VO")))
    .bind(sqlx::types::Json(svg_picture("banner.svg", 1600, 400, color, "Pixel art")))
    .bind(sqlx::types::Json(works))
    .bind(format!("e2e-{handle}"))
    .execute(&ctx.db)
    .await
    .context("portfolio")?;
    Ok(set)
}

/// A creative's People profile (PUT /v1/me/creative), taking work.
async fn creative_profile(ctx: &Ctx, who: &Who, handle: &str, name: &str) -> anyhow::Result<()> {
    ctx.put(
        who,
        "/v1/me/creative",
        json!({
            "handle": handle,
            "name": name,
            "kind": "individual",
            "headline": "Pixel art and tiny animations",
            "about_md": "I draw **sprites** and tiles for small games.",
            "skills": ["pixel-art", "music"],
            "ai_use": "none",
            "trader": true,
            "taking_work": true,
            "turnaround": "1–2 weeks",
            "accent": "grape",
            "sections": ["services", "portfolio"],
            "links": [{ "label": "", "url": "https://www.artstation.com/e2e" }],
            "services": [
                { "name": "Sprite sheet", "description": "One character, 4 directions", "price_minor": 12000, "currency": "usd", "days": 7, "skill": "pixel-art" }
            ],
            "confirm_adult": true,
        }),
    )
    .await?;
    Ok(())
}

pub struct People {
    pub creative: Who,
    pub client: Who,
    pub pending_creative: Who,
    /// Delivered, with a conversation: the client is asked to pay.
    pub delivered_job: Uuid,
    /// Paid up front through (fake) Stripe Checkout, and being worked on.
    pub working_job: Uuid,
    /// Just asked for.
    pub requested_job: Uuid,
    /// Accepted; the client opened a problem.
    pub problem_job: Uuid,
    /// Completed and paid; its review is disputed.
    pub completed_job: Uuid,
}

/// A live creative (Vera, "vera-pixels") hired by a client (Cleo) for their app "cleos-tavern".
async fn people(ctx: &Ctx, admin: &Who) -> anyhow::Result<People> {
    let creative = ctx.user("Vera Okafor", "US").await?;
    creative_profile(ctx, &creative, "vera-pixels", "Vera Okafor").await?;
    let set = portfolio(ctx, creative.id, "vera-pixels", "#6b3fa0").await?;
    ctx.post(
        admin,
        &format!("/v1/admin/portfolio/{set}/approve"),
        json!({}),
    )
    .await?;
    // Getting paid: Stripe's sign-up (the fake's), finished at once.
    ctx.call(
        Some(&creative),
        Method::POST,
        "/v1/me/creative/stripe",
        None,
    )
    .await?;
    let acct: String =
        sqlx::query_scalar("select stripe_account_id from creatives where user_id = $1")
            .bind(creative.id)
            .fetch_one(&ctx.db)
            .await?;
    ctx.stripe.activate(&acct).await?;
    let me = ctx.get(Some(&creative), "/v1/me/creative").await?;
    anyhow::ensure!(
        sqlx::query_scalar::<_, bool>(
            "select stripe_status = 'active' and portfolio_set_id is not null from creatives where user_id = $1"
        )
        .bind(creative.id)
        .fetch_one(&ctx.db)
        .await?,
        "the creative isn't live: {me}"
    );

    // A creative whose first portfolio waits for staff (the portfolio queue).
    let pending_creative = ctx.user("Remy Pending", "US").await?;
    creative_profile(ctx, &pending_creative, "remy-sounds", "Remy Pending").await?;
    portfolio(ctx, pending_creative.id, "remy-sounds", "#1f6f8b").await?;

    // The client and their app.
    let client = ctx.adult("Cleo Client", "US").await?;
    listed(ctx, client.id, "cleos-tavern", "Cleo's Tavern", "").await?;
    let hire = |request: &str, inquiry: bool| {
        json!({
            "app_slug": "cleos-tavern", "skill": "pixel-art", "service_id": null,
            "request_md": request, "confirm_adult": true, "exact_wording_ack": true, "inquiry": inquiry,
        })
    };
    let offer = |title: &str, payment: &str| {
        json!({
            "title": title,
            "scope_md": "4 directions × 8 frames, 32×32, PNG and Aseprite files.",
            "price_minor": 12000,
            "currency": "usd",
            "payment": payment,
            "due_date": (time::OffsetDateTime::now_utc().date() + time::Duration::days(14)).to_string(),
            "revisions": 1,
            "rights": "transfer",
            "open_source": false,
            "credit": true,
            "ai_use": "none",
        })
    };
    let job_id =
        |v: &Value| -> anyhow::Result<Uuid> { Ok(v["id"].as_str().context("job id")?.parse()?) };

    // 1. Asked about first, talked over, offered, accepted, delivered: waiting for the client.
    let job = ctx
        .post(
            &client,
            "/v1/people/vera-pixels/hire",
            hire("A knight for the tavern door, 4 directions, 32×32.", true),
        )
        .await?;
    let delivered_job = job_id(&job)?;
    let j = format!("/v1/jobs/{delivered_job}");
    ctx.post(&creative, &format!("{j}/conversation/open"), json!({}))
        .await?;
    ctx.post(
        &creative,
        &format!("{j}/messages"),
        json!({ "body": "Happy to! Which engine, and do you want an idle animation too?", "recording_ack": true }),
    )
    .await?;
    ctx.post(
        &client,
        &format!("{j}/messages"),
        json!({ "body": "Godot 4. Idle would be lovely, if it fits the price.", "recording_ack": true }),
    )
    .await?;
    ctx.post(
        &creative,
        &format!("{j}/messages"),
        json!({ "body": "It fits. Sending the offer now." }),
    )
    .await?;
    ctx.post(
        &creative,
        &format!("{j}/offer"),
        offer("Knight sprite sheet", "on_delivery"),
    )
    .await?;
    ctx.post(
        &client,
        &format!("{j}/accept"),
        json!({ "offer_count": 1, "exact_terms_ack": true }),
    )
    .await?;
    ctx.post(
        &creative,
        &format!("{j}/deliver"),
        json!({ "links": ["https://drive.example/knight-preview.zip"], "note": "Preview attached; full files after payment." }),
    )
    .await?;

    // 2. Paid up front through Stripe Checkout: being worked on.
    let job = ctx
        .post(
            &client,
            "/v1/people/vera-pixels/hire",
            hire("Twelve tavern floor tiles that repeat.", false),
        )
        .await?;
    let working_job = job_id(&job)?;
    let j = format!("/v1/jobs/{working_job}");
    ctx.post(
        &creative,
        &format!("{j}/offer"),
        offer("Tavern floor tiles", "upfront"),
    )
    .await?;
    ctx.post(
        &client,
        &format!("{j}/accept"),
        json!({ "offer_count": 1, "exact_terms_ack": true }),
    )
    .await?;
    pay(
        ctx,
        &ctx.post(&client, &format!("{j}/pay"), json!({})).await?,
    )
    .await?;
    let status = ctx.get(Some(&client), &j).await?;
    anyhow::ensure!(
        status["status"] == "in_progress",
        "the paid job isn't in progress: {status}"
    );

    // 3. Just asked for: the creative's to answer.
    let job = ctx
        .post(
            &client,
            "/v1/people/vera-pixels/hire",
            hire("Three tavern table props: a mug, a candle, a plate.", false),
        )
        .await?;
    let requested_job = job_id(&job)?;

    // 4. Accepted, then the client opened a problem (the People problems queue).
    let job = ctx
        .post(
            &client,
            "/v1/people/vera-pixels/hire",
            hire("A barmaid, idle and walking, 32×32.", false),
        )
        .await?;
    let problem_job = job_id(&job)?;
    let j = format!("/v1/jobs/{problem_job}");
    ctx.post(
        &creative,
        &format!("{j}/offer"),
        offer("Barmaid sprites", "on_delivery"),
    )
    .await?;
    ctx.post(
        &client,
        &format!("{j}/accept"),
        json!({ "offer_count": 1, "exact_terms_ack": true }),
    )
    .await?;
    ctx.post(
        &client,
        &format!("{j}/problem"),
        json!({ "statement": "Nothing has arrived and Vera stopped answering a week ago." }),
    )
    .await?;

    // 5. Done: delivered, paid on delivery through Stripe Checkout, final files sent. The
    // client's review is unfair and Vera disputes it (the review disputes queue).
    let job = ctx
        .post(
            &client,
            "/v1/people/vera-pixels/hire",
            hire("A tavern sign with the name painted on.", false),
        )
        .await?;
    let completed_job = job_id(&job)?;
    let j = format!("/v1/jobs/{completed_job}");
    ctx.post(
        &creative,
        &format!("{j}/offer"),
        offer("Tavern sign", "on_delivery"),
    )
    .await?;
    ctx.post(
        &client,
        &format!("{j}/accept"),
        json!({ "offer_count": 1, "exact_terms_ack": true }),
    )
    .await?;
    ctx.post(
        &creative,
        &format!("{j}/deliver"),
        json!({ "links": ["https://drive.example/sign-preview.png"] }),
    )
    .await?;
    pay(
        ctx,
        &ctx.post(&client, &format!("{j}/pay"), json!({})).await?,
    )
    .await?;
    ctx.post(
        &creative,
        &format!("{j}/final-files"),
        json!({ "links": ["https://drive.example/sign-final.zip"] }),
    )
    .await?;
    let done = ctx.get(Some(&client), &j).await?;
    anyhow::ensure!(done["status"] == "completed", "job 5 isn't done: {done}");
    ctx.put(
        &client,
        &format!("{j}/review"),
        json!({ "vote": "slop", "body": "Make me a second sign for free or this review stays." }),
    )
    .await?;
    ctx.post(
        &creative,
        &format!("/v1/people/reviews/{completed_job}/dispute"),
        json!({ "reason": "extortion", "statement": "The client wants free work in exchange for removing this." }),
    )
    .await?;

    Ok(People {
        creative,
        client,
        pending_creative,
        delivered_job,
        working_job,
        requested_job,
        problem_job,
        completed_job,
    })
}

// ---------------------------------------------------------------------------------------------
// Store and publishing

/// A release waiting for review, as the fetcher leaves one: its file fetched (the fake GitHub's
/// zip), hashed and scanned. `platform` "any" for a mod (no program to start).
async fn release_for_review(
    ctx: &Ctx,
    app: Uuid,
    slug: &str,
    version: &str,
    by: Uuid,
    platform: &str,
) -> anyhow::Result<Uuid> {
    use sha2::Digest as _;
    let zip = crate::release_zip();
    let id = Uuid::now_v7();
    ctx.sql(
        "insert into releases (id, app_id, version, commit_sha, status, submitted_by, commit_verified)
         values ($1, $2, $3, repeat('b', 40), 'awaiting_review', $4, true)",
        &[&id, &app, &version, &by],
    )
    .await?;
    ctx.sql(
        "insert into release_files (release_id, platform, kind, url, file_name, exec, size_bytes, sha256, scan_result)
         values ($1, $2, 'archive', $3, $4, $5, $6, $7, 'passed')",
        &[
            &id,
            &platform,
            &format!(
                "{}e2e/{slug}/releases/download/v{version}/{slug}.zip",
                ctx.fake
            ),
            &format!("{slug}.zip"),
            &if platform == "any" {
                None::<String>
            } else {
                Some("pong.exe".to_string())
            },
            &(zip.len() as i64),
            &hex::encode(sha2::Sha256::digest(&zip)),
        ],
    )
    .await?;
    Ok(id)
}

/// A release approved (and signed) by staff, through the review queue.
async fn approved_release(
    ctx: &Ctx,
    admin: &Who,
    app: Uuid,
    slug: &str,
    version: &str,
    by: Uuid,
    platform: &str,
) -> anyhow::Result<Uuid> {
    let id = release_for_review(ctx, app, slug, version, by, platform).await?;
    ctx.post(
        admin,
        &format!("/v1/admin/releases/{id}/approve"),
        json!({}),
    )
    .await?;
    Ok(id)
}

/// A store page made by its maker (POST /v1/apps: a draft).
#[allow(clippy::too_many_arguments)]
async fn new_app(
    ctx: &Ctx,
    maker: &Who,
    slug: &str,
    title: &str,
    summary: &str,
    stage: &str,
    tags: &[&str],
    platforms: &[&str],
) -> anyhow::Result<Uuid> {
    ctx.post(
        maker,
        "/v1/apps",
        json!({
            "slug": slug,
            "title": title,
            "summary": summary,
            "description_md": format!("# {title}\n\n{summary}\n\nMade for Slop's browser tests."),
            "kind": "game",
            "stage": stage,
            "repo_url": format!("https://github.com/e2e/{slug}"),
            "license_spdx": "MIT",
            "min_age": 12,
            "tags": tags,
            "platforms": platforms,
        }),
    )
    .await
    .with_context(|| format!("new app {slug}"))?;
    ctx.app_id(slug).await
}

/// A maker's app with a signed release, listed by staff.
#[allow(clippy::too_many_arguments)]
async fn published(
    ctx: &Ctx,
    admin: &Who,
    maker: &Who,
    slug: &str,
    title: &str,
    summary: &str,
    tags: &[&str],
) -> anyhow::Result<Uuid> {
    let id = new_app(
        ctx,
        maker,
        slug,
        title,
        summary,
        "released",
        tags,
        &["windows"],
    )
    .await?;
    approved_release(ctx, admin, id, slug, "1.0.0", maker.id, "windows").await?;
    ctx.post(admin, &format!("/v1/admin/apps/{slug}/list"), json!({}))
        .await?;
    Ok(id)
}

/// Turns on donations to `slug` with the maker's own (fake) Stripe account, finished at once.
async fn donations_on(ctx: &Ctx, maker: &Who, slug: &str, servers: bool) -> anyhow::Result<()> {
    ctx.post(
        maker,
        "/v1/me/donation-account",
        json!({ "tax_notice_ack": true, "confirm_adult": true, "return_to": format!("/publish/{slug}/manage#donations") }),
    )
    .await?;
    let acct: String =
        sqlx::query_scalar("select stripe_account_id from donation_accounts where user_id = $1")
            .bind(maker.id)
            .fetch_one(&ctx.db)
            .await?;
    ctx.stripe.activate(&acct).await?;
    ctx.put(
        maker,
        &format!("/v1/apps/{slug}/donations/settings"),
        json!({
            "donations_on": true,
            "donation_suggested_minor": 300,
            "donation_currency": "usd",
            "runs_servers": servers,
            "servers_statement": servers.then_some("Pays for two game servers and the leaderboard."),
        }),
    )
    .await?;
    Ok(())
}

/// Pays a Checkout link Slop gave (the fake Stripe's page), as the payer would.
async fn pay(ctx: &Ctx, answer: &Value) -> anyhow::Result<()> {
    let url = answer["url"].as_str().context("no checkout link")?;
    let session = url.rsplit('/').next().context("session id")?;
    ctx.stripe.pay(session).await?;
    Ok(())
}

/// A player who has played `slug` through Slop (needed to vote, review and report bugs).
async fn launch(ctx: &Ctx, who: &Who, slug: &str) -> anyhow::Result<()> {
    ctx.call(
        Some(who),
        Method::POST,
        &format!("/v1/apps/{slug}/launch?platform=windows"),
        None,
    )
    .await?;
    Ok(())
}

/// Who the store section needs.
pub struct StoreCast<'a> {
    pub admin: &'a Who,
    pub maker: &'a Who,
    pub maker2: &'a Who,
    pub player: &'a Who,
    pub client: &'a Who,
    pub teen: &'a Who,
    pub fans: &'a [Who],
}

/// Mia's "Lantern Keep" (everything a busy store page has), Bo's apps (a release and pictures
/// waiting for review, a repository move, a frozen and a delisted one), a community build and a
/// mod of Lantern Keep, a draft, bug reports and bounties.
async fn store(
    ctx: &Ctx,
    c: &StoreCast<'_>,
    ids: &mut BTreeMap<&'static str, Value>,
) -> anyhow::Result<()> {
    let admin = c.admin;
    // Lantern Keep: signed release, pictures approved by staff.
    let lantern = published(
        ctx,
        admin,
        c.maker,
        "lantern-keep",
        "Lantern Keep",
        "Light every lantern before the fog rolls in.",
        &["puzzle", "strategy"],
    )
    .await?;
    let media = ctx
        .call(
            Some(c.maker),
            Method::POST,
            "/v1/apps/lantern-keep/media/check",
            None,
        )
        .await?;
    let set = media["pending"]["id"]
        .as_str()
        .with_context(|| format!("no pictures waiting: {media}"))?;
    ctx.post(admin, &format!("/v1/admin/media/{set}/approve"), json!({}))
        .await?;
    ids.insert("listedApp", json!("lantern-keep"));
    ids.insert("listedAppId", json!(lantern));

    // Players: played it, voted, some wrote reviews; Mia answered one.
    let mut voters: Vec<(&Who, &str)> = vec![
        (c.player, "not_slop"),
        (c.client, "not_slop"),
        (c.maker2, "not_slop"),
        (c.teen, "not_slop"),
    ];
    for (i, fan) in c.fans.iter().enumerate() {
        voters.push((fan, if i == 0 { "slop" } else { "not_slop" }));
    }
    for (who, vote) in &voters {
        launch(ctx, who, "lantern-keep").await?;
        ctx.post(who, "/v1/apps/lantern-keep/vote", json!({ "vote": vote }))
            .await?;
    }
    let reviews = [
        (
            c.player,
            "The fog timer makes every level tense. Forty hours in and still finding secrets.",
        ),
        (c.teen, "Really fun puzzles, the music is great."),
        (&c.fans[0], "Crashed twice on level 3 and lost my save."),
    ];
    for (who, body) in reviews {
        ctx.call(
            Some(who),
            Method::PUT,
            "/v1/apps/lantern-keep/review",
            Some(json!({ "body": body })),
        )
        .await?;
    }
    let listed_reviews = ctx.get(None, "/v1/apps/lantern-keep/reviews").await?;
    let crash = listed_reviews["reviews"]
        .as_array()
        .or_else(|| listed_reviews.as_array())
        .and_then(|rs| {
            rs.iter()
                .find(|r| r["body"].as_str().is_some_and(|b| b.starts_with("Crashed")))
        })
        .and_then(|r| r["id"].as_str())
        .with_context(|| format!("the crash review: {listed_reviews}"))?
        .to_string();
    ctx.put(
        c.maker,
        &format!("/v1/apps/lantern-keep/reviews/{crash}/reply"),
        json!({ "body": "Sorry! 1.0.1 fixes the level 3 crash; your save should load again." }),
    )
    .await?;
    ids.insert("crashReview", json!(crash));

    // Bug reports: two open (one with a "me too"), one fixed.
    let bug = ctx
        .post(
            c.player,
            "/v1/apps/lantern-keep/bug-reports",
            json!({ "title": "Fog covers the menu after alt-tab", "body": "Alt-tab out and back: the fog is drawn over the pause menu.", "version": "1.0.0", "platform": "windows" }),
        )
        .await?;
    let bug_id = bug["id"].as_str().context("bug id")?.to_string();
    ctx.call(
        Some(&c.fans[0]),
        Method::POST,
        &format!("/v1/bug-reports/{bug_id}/me-too"),
        None,
    )
    .await?;
    let crash_bug = ctx
        .post(
            &c.fans[0],
            "/v1/apps/lantern-keep/bug-reports",
            json!({ "title": "Crash on level 3", "body": "Lighting the third lantern crashes the game.", "version": "1.0.0", "platform": "windows", "log": "thread 'main' panicked at src/level.rs:88" }),
        )
        .await?;
    let crash_bug = crash_bug["id"].as_str().context("bug id")?.to_string();
    ctx.call(
        Some(c.maker),
        Method::PATCH,
        &format!("/v1/bug-reports/{crash_bug}"),
        Some(json!({ "status": "fixed", "maker_note": "Fixed in 1.0.1." })),
    )
    .await?;
    ctx.post(
        c.teen,
        "/v1/apps/lantern-keep/bug-reports",
        json!({ "title": "Text too small on the map", "body": "The map labels are hard to read." }),
    )
    .await?;
    ids.insert("bugReport", json!(bug_id));
    ids.insert("fixedBugReport", json!(crash_bug));

    // Donations: Mia's Stripe account is active; it takes one-off and server donations.
    donations_on(ctx, c.maker, "lantern-keep", true).await?;
    let started = ctx
        .post(
            c.player,
            "/v1/apps/lantern-keep/donate",
            json!({ "amount_minor": 500, "monthly": false, "share_name": true, "message": "Thanks for the fog!", "consent": false, "confirm_adult": true }),
        )
        .await?;
    pay(ctx, &started).await?;
    ids.insert("donation", started["id"].clone());
    let started = ctx
        .post(
            c.client,
            "/v1/apps/lantern-keep/donate",
            json!({ "amount_minor": 400, "monthly": true, "share_name": false, "message": null, "consent": true, "confirm_adult": true }),
        )
        .await?;
    pay(ctx, &started).await?;
    ids.insert("serverDonation", started["id"].clone());

    // Bo's own app: a new release and new pictures wait for staff, and a repository move does too
    // (players have it, so it's checked by a person).
    let golf = published(
        ctx,
        admin,
        c.maker2,
        "pocket-golf",
        "Pocket Golf",
        "Mini golf on a pocket-sized course.",
        &["simulation"],
    )
    .await?;
    let waiting =
        release_for_review(ctx, golf, "pocket-golf", "1.1.0", c.maker2.id, "windows").await?;
    ctx.call(
        Some(c.maker2),
        Method::POST,
        "/v1/apps/pocket-golf/media/check",
        None,
    )
    .await?;
    launch(ctx, c.player, "pocket-golf").await?;
    ctx.post(
        c.maker2,
        "/v1/apps/pocket-golf/changes",
        json!({ "kind": "repo", "repo_url": "https://github.com/e2e/pocket-golf-2", "note": "Moved to the studio's organisation." }),
    )
    .await?;
    // A re-scan of its live release found something (the re-scan queue).
    ctx.sql(
        "update release_files set rescan_result = 'changed', rescan_flagged_at = now()
         where release_id = (select id from releases where app_id = $1 and version = '1.0.0')",
        &[&golf],
    )
    .await?;
    ids.insert("maker2App", json!("pocket-golf"));
    ids.insert("releaseInReview", json!(waiting));
    ids.insert("releaseInReviewApp", json!("pocket-golf"));

    // Bo's community build and mod of Lantern Keep, through the register-a-branch form's route.
    ctx.post(
        c.maker2,
        "/v1/apps/lantern-keep/branches",
        json!({
            "slug": "lantern-keep-linux", "title": "Lantern Keep for Linux",
            "summary": "Lantern Keep, built for Linux.", "repo_url": "https://github.com/e2e/lantern-keep-linux",
            "license_spdx": "MIT", "kind": "port", "note": "Adds a native Linux build.",
            "platforms": ["linux", "linux-deb"],
        }),
    )
    .await?;
    let build = ctx.app_id("lantern-keep-linux").await?;
    approved_release(
        ctx,
        admin,
        build,
        "lantern-keep-linux",
        "1.0.0",
        c.maker2.id,
        "linux",
    )
    .await?;
    ctx.post(admin, "/v1/admin/apps/lantern-keep-linux/list", json!({}))
        .await?;
    ctx.post(
        c.maker2,
        "/v1/apps/lantern-keep/branches",
        json!({
            "slug": "lantern-keep-more-lanterns", "title": "More Lanterns",
            "summary": "Twelve new lantern puzzles for Lantern Keep.", "repo_url": "https://github.com/e2e/more-lanterns",
            "license_spdx": "MIT", "kind": "addon", "note": "Twelve new puzzles and a harder fog.",
        }),
    )
    .await?;
    let lanterns_mod = ctx.app_id("lantern-keep-more-lanterns").await?;
    approved_release(
        ctx,
        admin,
        lanterns_mod,
        "lantern-keep-more-lanterns",
        "1.0.0",
        c.maker2.id,
        "any",
    )
    .await?;
    ctx.post(
        admin,
        "/v1/admin/apps/lantern-keep-more-lanterns/list",
        json!({}),
    )
    .await?;
    ids.insert("communityBuild", json!("lantern-keep-linux"));
    ids.insert("mod", json!("lantern-keep-more-lanterns"));

    // Mia's draft: coming soon, not in the store yet (staff's "Ready for the store").
    new_app(
        ctx,
        c.maker,
        "starship-draft",
        "Starship Draft",
        "A space trading game, not out yet.",
        "coming_soon",
        &["strategy"],
        &["windows", "linux", "linux-deb"],
    )
    .await?;
    ids.insert("draftApp", json!("starship-draft"));

    // Frozen by staff, and taken out of the store by staff.
    published(
        ctx,
        admin,
        c.maker2,
        "frozen-fjord",
        "Frozen Fjord",
        "Ice fishing at the end of the world.",
        &["simulation"],
    )
    .await?;
    ctx.post(
        admin,
        "/v1/admin/apps/frozen-fjord/freeze",
        json!({ "reason": "Checking a report that the last build mines cryptocurrency." }),
    )
    .await?;
    published(
        ctx,
        admin,
        c.maker2,
        "delisted-dash",
        "Delisted Dash",
        "A racing game that promised more than it had.",
        &["racing"],
    )
    .await?;
    ctx.call(
        Some(admin),
        Method::POST,
        "/v1/admin/apps/delisted-dash/delist",
        Some(json!({ "rule": "misleading", "reason": "The store page shows features the game doesn't have." })),
    )
    .await?;
    ids.insert("frozenApp", json!("frozen-fjord"));
    ids.insert("delistedApp", json!("delisted-dash"));

    // Bounties on Lantern Keep: one open with a pledge, one claimed by Bo's Linux build.
    // Pledges save a card through (fake) Stripe Checkout; claiming needs Bo's own Stripe account.
    let open = ctx
        .post(
            c.player,
            "/v1/apps/lantern-keep/bounties",
            json!({ "title": "Controller support", "body": "Let me play with a gamepad.", "currency": "usd" }),
        )
        .await?;
    let open_id = open["id"].as_str().context("bounty id")?.to_string();
    let pledged = ctx
        .post(
            c.player,
            &format!("/v1/bounties/{open_id}/pledge"),
            json!({ "amount_minor": 1500, "confirm_adult": true }),
        )
        .await?;
    pay(ctx, &pledged).await?;
    let claimed = ctx
        .post(
            c.client,
            "/v1/apps/lantern-keep/bounties",
            json!({ "title": "A Linux build", "body": "Please make it run on Linux.", "currency": "usd" }),
        )
        .await?;
    let claimed_id = claimed["id"].as_str().context("bounty id")?.to_string();
    let pledged = ctx
        .post(
            c.client,
            &format!("/v1/bounties/{claimed_id}/pledge"),
            json!({ "amount_minor": 2500, "confirm_adult": true }),
        )
        .await?;
    pay(ctx, &pledged).await?;
    let fans_pledge = ctx
        .post(
            &c.fans[1],
            &format!("/v1/bounties/{claimed_id}/pledge"),
            json!({ "amount_minor": 1000, "confirm_adult": true }),
        )
        .await?;
    pay(ctx, &fans_pledge).await?;
    builder_account(ctx, c.maker2).await?;
    ctx.post(
        c.maker2,
        &format!("/v1/bounties/{claimed_id}/claims"),
        json!({ "app_slug": "lantern-keep-linux", "note": "Lantern Keep for Linux 1.0.0 builds and runs on Linux." }),
    )
    .await?;
    for (id, want) in [(&open_id, "open"), (&claimed_id, "claimed")] {
        let b = ctx
            .get(Some(c.player), &format!("/v1/bounties/{id}"))
            .await?;
        anyhow::ensure!(b["status"] == want, "bounty {id} isn't {want}: {b}");
    }
    ids.insert("openBounty", json!(open_id));
    ids.insert("claimedBounty", json!(claimed_id));
    Ok(())
}

/// A builder's own Stripe account (to be paid bounties), finished at once.
async fn builder_account(ctx: &Ctx, who: &Who) -> anyhow::Result<()> {
    ctx.post(
        who,
        "/v1/me/donation-account",
        json!({ "tax_notice_ack": true, "confirm_adult": true }),
    )
    .await?;
    let acct: String =
        sqlx::query_scalar("select stripe_account_id from donation_accounts where user_id = $1")
            .bind(who.id)
            .fetch_one(&ctx.db)
            .await?;
    ctx.stripe.activate(&acct).await
}

// ---------------------------------------------------------------------------------------------
// People and social

/// A user's Slop desktop app with chat, signed in through the desktop sign-in flow, as the API
/// tests make them (tests/api/main.rs `chat_app`). Its keys live only in memory and a temporary
/// folder: nobody can read the seeded messages later, as with any device that's gone.
async fn chat_app(
    ctx: &Ctx,
    who: &Who,
    dir: &std::path::Path,
) -> anyhow::Result<slop_client::chat::Chat> {
    use slop_client::api::TokenStore as _;
    let verifier = "v".repeat(43);
    let created = ctx
        .call(
            None,
            Method::POST,
            "/v1/auth/desktop/requests",
            Some(json!({
                "code_challenge": slop_api::tokens::pkce_challenge(&verifier),
                "redirect_port": 49152,
                "state": "state-state-state-1",
                "device_name": "Slop for Linux",
            })),
        )
        .await?;
    let id = created["id"].as_str().context("desktop request id")?;
    let approved = ctx
        .post(
            who,
            &format!("/v1/auth/desktop/requests/{id}/approve"),
            json!({}),
        )
        .await?;
    let redirect = url::Url::parse(approved["redirect_url"].as_str().context("redirect")?)?;
    let code = redirect
        .query_pairs()
        .find(|(k, _)| k == "code")
        .context("code")?
        .1
        .into_owned();
    let tokens = ctx
        .call(
            None,
            Method::POST,
            "/v1/auth/desktop/token",
            Some(json!({ "grant_type": "authorization_code", "code": code, "code_verifier": verifier })),
        )
        .await?;
    let store = slop_client::api::MemoryTokenStore::default();
    store.save(tokens["refresh_token"].as_str().context("refresh token")?);
    let api = slop_client::api::ApiClient::new(
        ctx.config.public_url.clone(),
        std::sync::Arc::new(store),
    )?;
    Ok(slop_client::chat::Chat::open(
        api,
        dir.join(who.id.to_string()),
        &slop_client::chat::MemorySecrets::default(),
        who.id,
    )?)
}

pub struct SocialCast<'a> {
    pub player: &'a Who,
    pub maker2: &'a Who,
    pub client: &'a Who,
    pub teen: &'a Who,
    pub maker: &'a Who,
    pub creative: &'a Who,
    pub admin: &'a Who,
}

/// Friends, two teams (one invite-only with a team chat, one open), a direct chat, and unread
/// notifications for most accounts.
async fn social(
    ctx: &Ctx,
    c: &SocialCast<'_>,
    ids: &mut BTreeMap<&'static str, Value>,
) -> anyhow::Result<()> {
    // Friends: Pat and Bo, Pat and Tia; Cleo has asked Pat (waiting).
    for (from, to) in [(c.player, c.maker2), (c.player, c.teen)] {
        ctx.post(from, "/v1/me/friends", json!({ "user": to.id }))
            .await?;
        ctx.call(
            Some(to),
            Method::POST,
            &format!("/v1/me/friends/{}/accept", from.id),
            None,
        )
        .await?;
    }
    ctx.post(c.client, "/v1/me/friends", json!({ "user": c.player.id }))
        .await?;

    // Night Owls: invite-only, Pat's; Bo and Cleo joined, Tia is invited.
    ctx.post(
        c.player,
        "/v1/teams",
        json!({ "slug": "night-owls", "name": "Night Owls", "kind": "player", "about": "We play co-op games on Friday nights." }),
    )
    .await?;
    for who in [c.maker2, c.client] {
        ctx.post(
            c.player,
            "/v1/teams/night-owls/invites",
            json!({ "user": format!("/u/{}", who.id) }),
        )
        .await?;
        ctx.call(
            Some(who),
            Method::POST,
            "/v1/teams/night-owls/invites/accept",
            None,
        )
        .await?;
    }
    ctx.post(
        c.player,
        "/v1/teams/night-owls/invites",
        json!({ "user": format!("/u/{}", c.teen.id) }),
    )
    .await?;
    ctx.post(
        c.player,
        "/v1/teams/night-owls/picks",
        json!({ "app_slug": "lantern-keep", "note": "Great with four, everyone takes a lantern." }),
    )
    .await?;
    let friday = time::OffsetDateTime::now_utc() + time::Duration::days(3);
    ctx.post(
        c.maker2,
        "/v1/teams/night-owls/events",
        json!({
            "title": "Lantern night",
            "starts_at": friday.format(&time::format_description::well_known::Rfc3339)?,
            "app_slug": "lantern-keep",
        }),
    )
    .await?;
    // Open Crew: anyone can join; Cleo did.
    ctx.post(
        c.maker2,
        "/v1/teams",
        json!({ "slug": "open-crew", "name": "Open Crew", "kind": "player", "join_policy": "open", "about": "Anyone welcome." }),
    )
    .await?;
    ctx.call(
        Some(c.client),
        Method::POST,
        "/v1/teams/open-crew/join",
        None,
    )
    .await?;
    ids.insert("inviteTeam", json!("night-owls"));
    ids.insert("openTeam", json!("open-crew"));

    // Chat, end-to-end encrypted, from the desktop app: Pat and Bo directly, and the Night
    // Owls' team chat. The devices are removed afterwards so a test's own device is the
    // account's first (no approval needed); the conversations stay.
    let dir = std::env::temp_dir().join(format!("slop-e2e-chat-{}", Uuid::now_v7()));
    let result = async {
        let pat = chat_app(ctx, c.player, &dir).await?;
        let bo = chat_app(ctx, c.maker2, &dir).await?;
        let cleo = chat_app(ctx, c.client, &dir).await?;
        for (app, name) in [
            (&pat, "Pat's PC"),
            (&bo, "Bo's laptop"),
            (&cleo, "Cleo's PC"),
        ] {
            app.setup(name).await?;
        }
        let direct = pat.start_direct(&c.maker2.id.to_string()).await?;
        pat.send(direct, "Lantern night on Friday?", None, vec![], false)
            .await?;
        bo.sync(0).await?;
        bo.send(
            direct,
            "Yes! I'll bring the Linux build.",
            None,
            vec![],
            false,
        )
        .await?;
        pat.sync(0).await?;
        let team = pat.start_team("night-owls").await?;
        bo.sync(0).await?;
        cleo.sync(0).await?;
        pat.send(team, "Who's in for Friday?", None, vec![], false)
            .await?;
        cleo.sync(0).await?;
        cleo.send(team, "Me! 8pm?", None, vec![], false).await?;
        for app in [&pat, &bo, &cleo] {
            app.sync(0).await?;
            if let Some(device) = app.status().await?.device_id {
                app.remove_device(device).await?;
            }
        }
        anyhow::Ok((direct, team))
    }
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    let (direct, team) = result.context("chat")?;
    ids.insert("directChat", json!(direct));
    ids.insert("teamChat", json!(team));

    // A few more unread notifications, as Slop's own jobs would send them (no route makes
    // these).
    let notices: [(&Who, &str, &str, &str, &str); 7] = [
        (
            c.player,
            "apps",
            "Lantern Keep 1.0.1 is out",
            "Fixes the level 3 crash.",
            "/app/lantern-keep",
        ),
        (c.player, "chat", "New message from Bo Builder", "", "/chat"),
        (
            c.teen,
            "apps",
            "Lantern Keep 1.0.1 is out",
            "Fixes the level 3 crash.",
            "/app/lantern-keep",
        ),
        (
            c.maker,
            "donations",
            "Pat Player donated $5.00 to Lantern Keep",
            "\"Thanks for the fog!\"",
            "/publish/lantern-keep/manage#donations",
        ),
        (
            c.maker2,
            "apps",
            "Pictures for Pocket Golf are waiting for review",
            "Staff look at new pictures before they show.",
            "/publish/pocket-golf",
        ),
        (
            c.creative,
            "jobs",
            "Cleo Client asked you for a job",
            "Three tavern table props.",
            "/people/jobs",
        ),
        (
            c.admin,
            "ops",
            "The nightly backup took twice as long",
            "It finished; nothing is lost.",
            "/admin",
        ),
    ];
    for (who, category, title, body, link) in notices {
        ctx.sql(
            "insert into notifications (id, user_id, category, title, body, link) values ($1, $2, $3, $4, $5, $6)",
            &[&Uuid::now_v7(), &who.id, &category, &title, &body, &link],
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Admin queues

/// Calls the API with an admin API token.
impl Ctx {
    pub async fn with_token(
        &self,
        token: &str,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> anyhow::Result<(axum::http::StatusCode, Value)> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, self.config.public_url.authority())
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        let body = match body {
            Some(json) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let res = self.router.clone().oneshot(builder.body(body)?).await?;
        let status = res.status();
        let bytes = res.into_body().collect().await?.to_bytes();
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }
}

pub struct AdminCast<'a> {
    pub admin: &'a Who,
    pub maker2: &'a Who,
    pub client: &'a Who,
    pub fan: &'a Who,
    pub suspended: &'a Who,
    pub sam: Uuid,
}

/// Open reports, appeals (a suspension and a delisting), a staff change waiting for a person,
/// a server alert and a possible-abuse signal.
async fn admin_queues(
    ctx: &Ctx,
    c: &AdminCast<'_>,
    ids: &mut BTreeMap<&'static str, Value>,
) -> anyhow::Result<()> {
    // Reports waiting for staff.
    let r1 = ctx
        .post(
            c.client,
            "/v1/reports",
            json!({ "kind": "app", "subject": "frozen-fjord", "reason": "malware", "details": "My antivirus flagged the installer and my CPU fan never stops.", "good_faith": true }),
        )
        .await?;
    ctx.post(
        c.fan,
        "/v1/reports",
        json!({ "kind": "app", "subject": "pocket-golf", "reason": "broken", "details": "The ball falls through the green on hole 4.", "good_faith": true }),
    )
    .await?;
    ctx.post(
        c.fan,
        "/v1/reports",
        json!({ "kind": "team", "subject": "open-crew", "reason": "spam", "details": "The team description links to a coin giveaway.", "good_faith": true }),
    )
    .await?;
    ids.insert("report", r1["reference"].clone());

    // Sid is suspended, and appeals.
    let decided = ctx
        .post(
            c.admin,
            &format!("/v1/admin/users/{}/decide", c.suspended.id),
            json!({ "action": "suspend_account", "rule": "harassment", "facts": "Sent insults to twelve players in a week.", "law": null }),
        )
        .await?;
    let decision = decided["decision"]
        .as_str()
        .context("decision")?
        .to_string();
    ctx.post(
        c.suspended,
        &format!("/v1/me/decisions/{decision}/appeal"),
        json!({ "appeal": "Someone else had my password that week; I've changed it." }),
    )
    .await?;
    // Bo appeals Delisted Dash being taken out of the store.
    let decisions = ctx.get(Some(c.maker2), "/v1/me/decisions").await?;
    let delist = decisions
        .as_array()
        .and_then(|d| d.first())
        .and_then(|d| d["id"].as_str())
        .with_context(|| format!("Bo's decisions: {decisions}"))?
        .to_string();
    ctx.post(
        c.maker2,
        &format!("/v1/me/decisions/{delist}/appeal"),
        json!({ "appeal": "Every feature on the page is in 1.0.0; here's a video of each." }),
    )
    .await?;
    ids.insert("suspensionDecision", json!(decision));
    ids.insert("delistDecision", json!(delist));

    // An AI agent with a moderate token asked to make Sam support staff: it waits for a person.
    let made = ctx
        .post(
            c.admin,
            "/v1/admin/tokens",
            json!({ "name": "Moderation agent", "scope": "moderate", "days": 7 }),
        )
        .await?;
    let token = made["token"].as_str().context("token")?;
    let (status, asked) = ctx
        .with_token(
            token,
            Method::POST,
            &format!("/v1/admin/staff/{}/roles", c.sam),
            Some(json!({ "role": "support", "grant": true })),
        )
        .await?;
    anyhow::ensure!(
        status == axum::http::StatusCode::ACCEPTED,
        "the staff change didn't wait: {status} {asked}"
    );
    ids.insert("staffAction", asked["pending_action"].clone());

    // What Slop's own checks would have found (no route makes these).
    ctx.sql(
        "insert into ops_alerts (key, title, detail, link) values ('disk-e2e', 'The disk is 91% full', 'Old release files could be cleaned up.', '/admin')",
        &[],
    )
    .await?;
    let golf = ctx.app_id("pocket-golf").await?;
    ctx.sql(
        "insert into abuse_signals (id, kind, key, subject_kind, subject_id, subject_label, detail)
         values ($1, 'vote_burst', 'vote_burst:pocket-golf', 'app', $2, 'Pocket Golf', '{}')",
        &[&Uuid::now_v7(), &golf],
    )
    .await?;
    Ok(())
}

/// Checks the seed ended where the tests expect: a query each, and the answer it must give.
async fn verify(ctx: &Ctx) -> anyhow::Result<()> {
    let checks: &[(&str, &str)] = &[
        (
            "select string_agg(status::text, ',' order by status::text) from donations",
            "paid,paid",
        ),
        (
            "select string_agg(status::text, ',') from server_donations where stripe_subscription_id is not null",
            "active",
        ),
        (
            "select string_agg(status::text, ',' order by status::text) from bounty_pledges where payment_method_id is not null",
            "active,active,active",
        ),
        (
            "select string_agg(status::text, ',' order by status::text) from bounties",
            "claimed,open",
        ),
        (
            "select string_agg(status::text, ',' order by status::text) from jobs",
            "completed,delivered,in_progress,in_progress,requested",
        ),
        (
            "select count(*)::text from apps where slug = 'lantern-keep' and media_set_id is not null and donations_on",
            "1",
        ),
        (
            "select string_agg(slug, ',' order by slug) from apps where frozen_at is not null",
            "frozen-fjord",
        ),
        (
            "select string_agg(slug || ':' || status::text, ',' order by slug) from apps where slug in ('delisted-dash', 'starship-draft', 'lantern-keep-linux', 'lantern-keep-more-lanterns')",
            "delisted-dash:delisted,lantern-keep-linux:listed,lantern-keep-more-lanterns:listed,starship-draft:draft",
        ),
        (
            "select count(*)::text from releases where status = 'awaiting_review'",
            "1",
        ),
        (
            "select string_agg(kind::text, ',' order by kind::text) from chat_conversations where kind <> 'self'",
            "direct,team",
        ),
        (
            "select count(*)::text from chat_devices where status <> 'revoked'",
            "0",
        ),
        (
            "select count(*)::text from moderation_decisions where appeal_status = 'pending'",
            "2",
        ),
        (
            "select count(*)::text from staff_actions where status = 'pending'",
            "1",
        ),
        (
            "select count(*)::text from reports where status = 'open'",
            "3",
        ),
        (
            "select string_agg(status::text, ',' order by status::text) from portfolio_sets",
            "approved,pending",
        ),
        (
            "select count(*)::text from media_sets where status = 'pending'",
            "1",
        ),
        (
            "select count(*)::text from app_changes where status = 'pending'",
            "1",
        ),
        (
            "select count(*)::text from job_problems where status = 'open'",
            "1",
        ),
        (
            "select count(*)::text from review_disputes where status = 'pending'",
            "1",
        ),
        ("select count(*)::text from admin_threads", "2"),
        ("select count(*)::text from donation_holds", "0"),
        ("select count(*)::text from store_shelves", "0"),
    ];
    let mut wrong = Vec::new();
    for (sql, want) in checks {
        let got: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
            .fetch_one(&ctx.db)
            .await
            .with_context(|| sql.to_string())?;
        if got.as_deref() != Some(want) {
            wrong.push(format!("{sql}\n  wanted {want}, got {got:?}"));
        }
    }
    anyhow::ensure!(
        wrong.is_empty(),
        "the seed isn't as expected:\n{}",
        wrong.join("\n")
    );
    Ok(())
}
