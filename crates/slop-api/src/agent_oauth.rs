// SPDX-License-Identifier: AGPL-3.0-or-later
//! AI tools that connect by being approved in the browser (SECURITY §8.4.3, DESIGN §8): the MCP
//! authorization spec (revision 2026-07-28), OAuth 2.1 with the RFCs it names. The person adds
//! SLAPorSLOP's MCP address to their AI tool; the tool finds out from the address where to ask,
//! opens SLAPorSLOP in the browser, the person signs in and chooses what it may do, and the tool
//! gets its tokens. Nobody copies a token anywhere.
//!
//! 1. **Discovery.** The MCP endpoint answers a request without a working token with 401 and
//!    `WWW-Authenticate: Bearer resource_metadata="…"` (maker_agents.rs). That document (RFC 9728)
//!    names SLAPorSLOP as the authorization server, whose metadata (RFC 8414) lists the endpoints
//!    below.
//! 2. **The tool's identity.** Either the https address of its own description (Client ID
//!    Metadata Documents, which the spec prefers), read from there with the same care as profile
//!    pictures: public addresses only, no redirects, 5 KB at most; or a registration (RFC 7591,
//!    kept for tools that only do that), limited per address and deleted when unused.
//! 3. **Approval.** `/v1/oauth/authorize` checks the tool and its return address exactly (any port
//!    for a loopback address, RFC 8252), requires a PKCE S256 challenge and an MCP address as the
//!    `resource` (RFC 8707), and sends the browser to the website's approval page
//!    (`/connect-ai/{id}`). There, signed in (signing in or up first, and coming back), the
//!    person sees the tool's name and, prominently, where it will send the browser, and chooses
//!    what it may do exactly as for a maker token: look or also publish (with the Publisher
//!    Agreement), all apps or some, People jobs, how long. Approve sends the browser back with a
//!    one-time code (and `iss`, RFC 9207); Deny with `access_denied`. No "confirm it's you": a
//!    signed-in session is enough (SECURITY §4.3).
//! 4. **Tokens.** The code, with the PKCE verifier, becomes a session of kind `agent` with a
//!    `maker_tokens` row, as if the person had made a token with those settings: the same
//!    allow-list, scope, apps and People checks apply unchanged, it's listed on Account → AI agents
//!    and the devices page, and removing it there ends it at once. The tool gets an access token
//!    (`slopmk_…`, an hour, never past the session's end) and a refresh token (`slopmr_…`) that
//!    changes on every use. A refresh token used twice was copied: the connection is ended
//!    (unless it's within `REUSE_GRACE` of its first use, when only the second request is
//!    refused: two copies of the same tool refreshing at once). A code used twice ends what the
//!    first use made.
//!
//! Tool names and descriptions are what the tool says about itself: the approval page always
//! shows where the browser will go, and warns when that's a website it doesn't know.

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration as StdDuration, Instant},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, RawQuery, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use url::Url;
use uuid::Uuid;

use crate::{
    AppState,
    auth::session::CurrentUser,
    error::{ApiError, ApiResult},
    maker_agents,
    tokens::{hash_token, new_token, pkce_challenge},
};

/// Refresh tokens start with this, so secret scanners can find them. Access tokens are maker
/// tokens (`maker_agents::PREFIX`), so the API reads them the same way.
pub const REFRESH_PREFIX: &str = "slopmr_";
/// How long an access token works. Removing a connection ends it at once whatever this is (every
/// request checks the session), so this only bounds a copied access token without its refresh
/// token; an hour keeps refreshes, and so the chance of two copies of a tool refreshing at once,
/// rare.
pub const ACCESS_LIFETIME: Duration = Duration::hours(1);
/// How long the person has to approve, from the tool opening the browser (reading the Publisher
/// Agreement included).
const REQUEST_LIFETIME: Duration = Duration::minutes(30);
/// How long the tool has to swap an approved code for tokens.
const CODE_LIFETIME: Duration = Duration::minutes(5);
/// A refresh token used again this soon after its first use is refused without ending the
/// connection: two copies of the same tool (two terminals) refreshing at the same moment. Either
/// way the second request gets nothing.
pub const REUSE_GRACE: Duration = Duration::seconds(30);
/// The scopes SLAPorSLOP knows. They only set where the approval page's choices start: the person
/// chooses there.
const SCOPES: &[&str] = &["read", "publish", "people"];
/// The scopes a tool asks for by default (the MCP spec: all of these, unless the 401 names some).
pub const DEFAULT_SCOPE: &str = "read publish";
/// The most a description may be (draft-ietf-oauth-client-id-metadata-document §8.7).
const MAX_DOCUMENT_BYTES: usize = 5 * 1024;
/// Registrations per address (IPv6: per /64) an hour.
const REGISTRATIONS_PER_HOUR: u32 = 20;
/// Registered tools that never started an approval, across SLAPorSLOP, before registering stops
/// for a while (each is deleted after a day).
const MAX_UNUSED_REGISTRATIONS: i64 = 2000;
/// A description is read again after this at the latest (or sooner, as its cache headers say).
const DOCUMENT_MAX_AGE: StdDuration = StdDuration::from_secs(24 * 3600);
const DOCUMENT_MIN_AGE: StdDuration = StdDuration::from_secs(60);
const DOCUMENT_DEFAULT_AGE: StdDuration = StdDuration::from_secs(3600);

/// Return addresses the approval page doesn't warn about: the AI services' own (from their
/// documentation and descriptions, checked 2026-10-27; see `web/src/agentSetup.ts`).
const KNOWN_WEB_HOSTS: &[&str] = &[
    "claude.ai",
    "claude.com",
    "chatgpt.com",
    "vscode.dev",
    "insiders.vscode.dev",
    "www.cursor.com",
];

// ---------------------------------------------------------------------------------------------
// Addresses

/// The site's address without the trailing slash: the issuer, and the base of the MCP addresses.
pub fn issuer(state: &AppState) -> String {
    state
        .config
        .public_url
        .as_str()
        .trim_end_matches('/')
        .to_string()
}

/// The protected resource metadata for an MCP endpoint's path (`/mcp` or `/v1/maker/mcp`).
pub fn resource_metadata_url(state: &AppState, mcp_path: &str) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource{mcp_path}",
        issuer(state)
    )
}

/// What the MCP endpoint answers without a working token: 401 saying where to find out how to
/// get one (RFC 9728 §5.1, RFC 6750 §3).
pub fn unauthorized(state: &AppState, mcp_path: &str, had_token: bool) -> Response {
    let mut res = ApiError::Unauthorized.into_response();
    let error = if had_token {
        "error=\"invalid_token\", "
    } else {
        ""
    };
    let value = format!(
        "Bearer {error}resource_metadata=\"{}\", scope=\"{DEFAULT_SCOPE}\"",
        resource_metadata_url(state, mcp_path)
    );
    if let Ok(v) = HeaderValue::from_str(&value) {
        res.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    res
}

/// The canonical form of a `resource` the tokens may be for: one of SLAPorSLOP's MCP addresses, or
/// the site itself (what the root metadata names). `None` for anything else.
fn canonical_resource(state: &AppState, resource: &str) -> Option<String> {
    let url = Url::parse(resource).ok()?;
    if url.fragment().is_some() || url.query().is_some() {
        return None;
    }
    let site = Url::parse(&issuer(state)).ok()?;
    if url.origin() != site.origin() {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    let base = issuer(state);
    match path {
        "" => Some(base),
        "/mcp" | "/v1/maker/mcp" => Some(format!("{base}{path}")),
        _ => None,
    }
}

/// Where a return address sends the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Return {
    /// A program on this computer (http://127.0.0.1, [::1] or localhost, any port).
    Loopback,
    /// A website, by its host.
    Web(String),
}

/// Checks a return address a tool registered or asked for (OAuth 2.1 §2.3, RFC 8252, and the MCP
/// spec's "either localhost or HTTPS"): https anywhere, http only to this computer. No
/// fragments, no user names. Apps' own schemes aren't accepted: the spec doesn't allow them, the
/// tools checked don't use them, and a browser handing a link to any app the computer has
/// (`ms-msdt:` and the like) is a risk of its own.
pub fn check_return(uri: &str) -> Result<Return, &'static str> {
    if uri.len() > 2000 {
        return Err("a redirect URI is too long");
    }
    let url = Url::parse(uri).map_err(|_| "a redirect URI isn't a valid absolute URI")?;
    if url.fragment().is_some() {
        return Err("a redirect URI can't have a fragment");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("a redirect URI can't contain a user name or password");
    }
    match url.scheme() {
        "https" => match url.host_str() {
            Some(h) if !h.is_empty() => Ok(Return::Web(h.to_ascii_lowercase())),
            _ => Err("a redirect URI needs a host"),
        },
        "http" if is_loopback(&url) => Ok(Return::Loopback),
        _ => Err("redirect URIs must use https, or http on this computer (127.0.0.1 or localhost)"),
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Whether `asked` is one of `registered`: exactly, except that a loopback address may use any
/// port (RFC 8252 §7.3; the tool picks a free one each time) and any of this computer's names
/// for itself (Claude Code has sent 127.0.0.1 for a registered localhost). The path and query
/// must still match exactly.
fn return_matches(registered: &[String], asked: &str) -> bool {
    if registered.iter().any(|r| r == asked) {
        return true;
    }
    let Ok(asked) = Url::parse(asked) else {
        return false;
    };
    if asked.scheme() != "http" || !is_loopback(&asked) || asked.fragment().is_some() {
        return false;
    }
    registered.iter().any(|r| {
        Url::parse(r).is_ok_and(|r| {
            r.scheme() == "http"
                && is_loopback(&r)
                && r.path() == asked.path()
                && r.query() == asked.query()
        })
    })
}

/// A tool's name as the page shows it: control and direction-changing characters taken out,
/// spaces tidied, at most 60 characters.
pub fn tidy_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let joined = words.join(" ");
    let short: String = joined.chars().take(60).collect();
    let short = short.trim().to_string();
    if short.is_empty() {
        "AI tool".into()
    } else {
        short
    }
}

/// Checks an address a tool names itself by (Client ID Metadata Documents §3): https, a path
/// other than `/`, no user name, fragment, query or `.`/`..` segments, and a host name, not an
/// IP address (tests may use http on this computer).
fn check_document_address(client_id: &str, allow_http: bool) -> Option<Url> {
    if client_id.len() > 500 {
        return None;
    }
    let url = Url::parse(client_id).ok()?;
    // The parsed form must be exactly what was sent: no normalising a different address into it.
    if url.as_str() != client_id {
        return None;
    }
    let scheme_ok = url.scheme() == "https" || (allow_http && url.scheme() == "http");
    let host_ok =
        matches!(url.host(), Some(url::Host::Domain(_))) || (allow_http && url.host().is_some());
    let dots = client_id
        .split(['/', '?'])
        .skip(3)
        .any(|seg| seg == "." || seg == "..");
    (scheme_ok
        && host_ok
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.query().is_none()
        && url.path() != "/"
        && !dots)
        .then_some(url)
}

// ---------------------------------------------------------------------------------------------
// Routes

/// The discovery documents and the short MCP address, outside `/v1`.
pub fn site_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_root),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(|s: State<AppState>| protected_resource(s, "/mcp")),
        )
        .route(
            "/.well-known/oauth-protected-resource/v1/maker/mcp",
            get(|s: State<AppState>| protected_resource(s, "/v1/maker/mcp")),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server),
        )
        // Other places tools look, answered plainly rather than with the website's page: there's
        // no OpenID Connect here, and no other resource.
        .route("/.well-known/openid-configuration", get(not_here))
        .route("/.well-known/openid-configuration/{*rest}", get(not_here))
        .route(
            "/.well-known/oauth-authorization-server/{*rest}",
            get(not_here),
        )
        .route(
            "/.well-known/oauth-protected-resource/{*rest}",
            get(not_here),
        )
}

async fn not_here() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// What AI tools call themselves, from anywhere (no cookies, so open to other origins).
pub fn client_routes() -> Router<AppState> {
    Router::new()
        .route("/oauth/register", post(register))
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
}

/// The browser's half: starting an approval, and the website's approval page.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/oauth/authorize", get(authorize))
        .route("/oauth/requests/{id}", get(request_info))
        .route("/oauth/requests/{id}/approve", post(approve))
        .route("/oauth/requests/{id}/deny", post(deny))
}

/// Open to tools running in a browser on another origin: nothing here uses cookies.
pub fn cors() -> tower_http::cors::CorsLayer {
    tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::HeaderName::from_static("mcp-protocol-version"),
        ])
}

/// The client endpoints (POST from other origins carries no cookies, so the CSRF check doesn't
/// apply to them).
pub fn is_client_endpoint(method: &Method, path: &str) -> bool {
    method == Method::POST
        && matches!(
            path,
            "/v1/oauth/register" | "/v1/oauth/token" | "/v1/oauth/revoke"
        )
}

// ---------------------------------------------------------------------------------------------
// Discovery

fn protected_resource_doc(state: &AppState, resource: String) -> Json<Value> {
    Json(json!({
        "resource": resource,
        "authorization_servers": [issuer(state)],
        "scopes_supported": DEFAULT_SCOPE.split(' ').collect::<Vec<_>>(),
        "bearer_methods_supported": ["header"],
        "resource_name": "SLAPorSLOP",
        "resource_documentation": format!("{}/publish/guide", issuer(state)),
    }))
}

async fn protected_resource(State(state): State<AppState>, path: &'static str) -> Json<Value> {
    let resource = format!("{}{path}", issuer(&state));
    protected_resource_doc(&state, resource)
}

/// For tools that look at the site's root: the resource is the site itself.
async fn protected_resource_root(State(state): State<AppState>) -> Json<Value> {
    let resource = issuer(&state);
    protected_resource_doc(&state, resource)
}

async fn authorization_server(State(state): State<AppState>) -> Json<Value> {
    let base = issuer(&state);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/v1/oauth/authorize"),
        "token_endpoint": format!("{base}/v1/oauth/token"),
        "registration_endpoint": format!("{base}/v1/oauth/register"),
        "revocation_endpoint": format!("{base}/v1/oauth/revoke"),
        "scopes_supported": SCOPES,
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
        "service_documentation": format!("{base}/publish/guide"),
    }))
}

// ---------------------------------------------------------------------------------------------
// Errors the tools read (RFC 6749 §5.2, RFC 7591 §3.2.2)

#[derive(Debug)]
struct OAuthError {
    status: StatusCode,
    code: &'static str,
    description: String,
}

fn oauth_error(code: &'static str, description: impl Into<String>) -> OAuthError {
    OAuthError {
        status: if code == "invalid_client" {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::BAD_REQUEST
        },
        code,
        description: description.into(),
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let mut res = (
            self.status,
            Json(json!({ "error": self.code, "error_description": self.description })),
        )
            .into_response();
        res.headers_mut()
            .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
        res
    }
}

impl From<sqlx::Error> for OAuthError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = ?err, "oauth endpoint");
        OAuthError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "server_error",
            description: "something went wrong".into(),
        }
    }
}

type OAuthResult<T> = Result<T, OAuthError>;

/// Form fields, refusing a field given twice (RFC 6749 §3.1).
fn form(body: &[u8]) -> OAuthResult<HashMap<String, String>> {
    let mut out = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(body) {
        if out.insert(k.to_string(), v.to_string()).is_some() {
            return Err(oauth_error(
                "invalid_request",
                format!("{k} is given twice"),
            ));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Registration (RFC 7591)

/// Registrations per address in the current hour (in memory, like `ratelimit.rs`).
#[derive(Default)]
pub struct Registrations {
    counts: Mutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl Registrations {
    /// Counts one registration from `ip`; false when it's over the hour's limit.
    fn take(&self, ip: IpAddr, now: Instant) -> bool {
        // An IPv6 home or server gets a whole /64: count the /64.
        let key = match ip {
            IpAddr::V6(v6) => {
                let mut o = v6.octets();
                o[8..].fill(0);
                IpAddr::from(o)
            }
            v4 => v4,
        };
        let mut counts = self.counts.lock().expect("registrations lock");
        if counts.len() > 50_000 {
            counts
                .retain(|_, (start, _)| now.duration_since(*start) < StdDuration::from_secs(3600));
        }
        let entry = counts.entry(key).or_insert((now, 0));
        if now.duration_since(entry.0) >= StdDuration::from_secs(3600) {
            *entry = (now, 0);
        }
        if entry.1 >= REGISTRATIONS_PER_HOUR {
            return false;
        }
        entry.1 += 1;
        true
    }
}

#[derive(Deserialize)]
struct RegistrationRequest {
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    response_types: Option<Vec<String>>,
}

async fn register(
    State(state): State<AppState>,
    request: Request,
) -> OAuthResult<(StatusCode, Json<Value>)> {
    let ip = crate::ratelimit::client_ip(&state, &request).unwrap_or(IpAddr::from([0, 0, 0, 0]));
    let body = axum::body::to_bytes(request.into_body(), 16 * 1024)
        .await
        .map_err(|_| oauth_error("invalid_client_metadata", "the request is too large"))?;
    let reg: RegistrationRequest = serde_json::from_slice(&body).map_err(|_| {
        oauth_error(
            "invalid_client_metadata",
            "send the client metadata as JSON",
        )
    })?;
    if reg.redirect_uris.is_empty() || reg.redirect_uris.len() > 10 {
        return Err(oauth_error(
            "invalid_redirect_uri",
            "give 1 to 10 redirect_uris",
        ));
    }
    for uri in &reg.redirect_uris {
        check_return(uri).map_err(|e| oauth_error("invalid_redirect_uri", e))?;
    }
    // Public clients only: a tool on someone's computer can't keep a secret, and PKCE protects
    // the code instead. Whatever `token_endpoint_auth_method` it asked for, it's told `none`
    // (RFC 7591 §3.2.1 lets SLAPorSLOP replace what it asked for).
    if let Some(grants) = &reg.grant_types
        && grants
            .iter()
            .any(|g| g != "authorization_code" && g != "refresh_token")
    {
        return Err(oauth_error(
            "invalid_client_metadata",
            "only the authorization_code and refresh_token grants are supported",
        ));
    }
    if let Some(types) = &reg.response_types
        && types.iter().any(|t| t != "code")
    {
        return Err(oauth_error(
            "invalid_client_metadata",
            "only the code response type is supported",
        ));
    }
    if !state.oauth_registrations.take(ip, Instant::now()) {
        return Err(OAuthError {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "invalid_client_metadata",
            description: "too many registrations from this address; try again later".into(),
        });
    }
    let unused: i64 = sqlx::query_scalar(
        "select count(*) from agent_clients where kind = 'registered' and last_used_at is null",
    )
    .fetch_one(&state.db)
    .await?;
    if unused >= MAX_UNUSED_REGISTRATIONS {
        return Err(OAuthError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "temporarily_unavailable",
            description: "registration is busy; try again in an hour".into(),
        });
    }
    let name = tidy_name(reg.client_name.as_deref().unwrap_or(""));
    let client_id = format!("slopcl_{}", new_token());
    let issued: OffsetDateTime = sqlx::query_scalar(
        "insert into agent_clients (id, client_id, kind, name, redirect_uris)
         values ($1, $2, 'registered', $3, $4) returning created_at",
    )
    .bind(Uuid::now_v7())
    .bind(&client_id)
    .bind(&name)
    .bind(&reg.redirect_uris)
    .fetch_one(&state.db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "client_id": client_id,
            "client_id_issued_at": issued.unix_timestamp(),
            "client_name": name,
            "redirect_uris": reg.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        })),
    ))
}

// ---------------------------------------------------------------------------------------------
// Knowing the tool

/// A tool SLAPorSLOP knows: its row, and the return addresses it may use.
struct Client {
    id: Uuid,
    redirect_uris: Vec<String>,
}

/// The tool a `client_id` names: a registered one, or one whose description is read from its
/// address (again, once the copy is older than its cache headers allow). `None` if there's no
/// such tool, or its description can't be read or doesn't hold.
async fn find_client(state: &AppState, client_id: &str) -> sqlx::Result<Option<Client>> {
    let row: Option<(Uuid, Vec<String>, bool)> = sqlx::query_as(
        "select id, redirect_uris,
                kind = 'document' and (refresh_after is null or refresh_after <= now())
         from agent_clients where client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(&state.db)
    .await?;
    if let Some((id, redirect_uris, stale)) = row
        && !stale
    {
        return Ok(Some(Client { id, redirect_uris }));
    }
    let allow_http = state.config.releases.allow_http_downloads;
    let Some(url) = check_document_address(client_id, allow_http) else {
        return Ok(None);
    };
    let Some((name, redirect_uris, max_age)) = read_document(state, &url).await else {
        return Ok(None);
    };
    // A registered tool's ID never looks like an address, so this only ever updates a
    // description read before.
    let row: Option<(Uuid, Vec<String>)> = sqlx::query_as(
        "insert into agent_clients (id, client_id, kind, name, redirect_uris, refresh_after)
         values ($1, $2, 'document', $3, $4, now() + $5)
         on conflict (client_id) do update
             set name = excluded.name, redirect_uris = excluded.redirect_uris,
                 refresh_after = excluded.refresh_after
             where agent_clients.kind = 'document'
         returning id, redirect_uris",
    )
    .bind(Uuid::now_v7())
    .bind(client_id)
    .bind(&name)
    .bind(&redirect_uris)
    .bind(Duration::try_from(max_age).unwrap_or(Duration::hours(1)))
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(|(id, redirect_uris)| Client { id, redirect_uris }))
}

/// Reads a tool's description from its address: public addresses only and no redirects (the
/// profile-picture client, avatars.rs), a 200 answer of at most 5 KB, JSON naming the same
/// address as `client_id`, with valid redirect URIs and no secrets. Returns its name, redirect
/// URIs and how long the copy may be kept.
async fn read_document(state: &AppState, url: &Url) -> Option<(String, Vec<String>, StdDuration)> {
    let mut res = state
        .avatar_http
        .get(url.as_str())
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .ok()?;
    if res.status() != StatusCode::OK {
        return None;
    }
    if res
        .content_length()
        .is_some_and(|n| n > MAX_DOCUMENT_BYTES as u64)
    {
        return None;
    }
    let max_age = cache_age(res.headers());
    let mut bytes = Vec::new();
    while let Some(chunk) = res.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return None;
        }
    }
    let doc: Value = serde_json::from_slice(&bytes).ok()?;
    let (name, uris) = check_document(&doc, url.as_str())?;
    Some((name, uris, max_age))
}

/// A description's name and redirect URIs, if it holds: it names `address` as its `client_id`,
/// has redirect URIs SLAPorSLOP accepts, and uses no shared secret (§4.1).
fn check_document(doc: &Value, address: &str) -> Option<(String, Vec<String>)> {
    if doc["client_id"].as_str() != Some(address) {
        return None;
    }
    if doc.get("client_secret").is_some() {
        return None;
    }
    // SLAPorSLOP lists only `none`, so the tool uses no client authentication with it (ChatGPT's
    // description prefers `private_key_jwt` but also supports `none`). A shared secret can't be
    // in a public document at all (§4.1).
    if doc["token_endpoint_auth_method"]
        .as_str()
        .is_some_and(|m| m.starts_with("client_secret"))
    {
        return None;
    }
    let uris: Vec<String> = doc["redirect_uris"]
        .as_array()?
        .iter()
        .map(|u| u.as_str().map(str::to_string))
        .collect::<Option<_>>()?;
    if uris.is_empty() || uris.len() > 10 || uris.iter().any(|u| check_return(u).is_err()) {
        return None;
    }
    let name = doc["client_name"]
        .as_str()
        .map(tidy_name)
        .unwrap_or_else(|| {
            Url::parse(address)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| "AI tool".into())
        });
    Some((name, uris))
}

/// How long a description may be kept, from `Cache-Control` (an hour if it doesn't say; within
/// a minute and a day).
fn cache_age(headers: &HeaderMap) -> StdDuration {
    let Some(cc) = headers
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
    else {
        return DOCUMENT_DEFAULT_AGE;
    };
    let mut age = DOCUMENT_DEFAULT_AGE;
    for part in cc.split(',').map(str::trim) {
        let lower = part.to_ascii_lowercase();
        if lower == "no-store" || lower == "no-cache" {
            age = DOCUMENT_MIN_AGE;
        } else if let Some(n) = lower.strip_prefix("max-age=")
            && let Ok(secs) = n.trim_matches('"').parse::<u64>()
        {
            age = StdDuration::from_secs(secs);
        }
    }
    age.clamp(DOCUMENT_MIN_AGE, DOCUMENT_MAX_AGE)
}

// ---------------------------------------------------------------------------------------------
// Starting an approval

/// The website's page explaining why an approval can't start (`/connect-ai?problem=…`): used
/// when the tool or its return address is wrong, so nothing may be sent back to it.
fn problem_page(state: &AppState, problem: &str) -> Response {
    Redirect::to(&format!("{}/connect-ai?problem={problem}", issuer(state))).into_response()
}

/// Sends the browser back to the tool with these answers, and `iss` (RFC 9207).
fn back_to_tool(state: &AppState, redirect_uri: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse(redirect_uri).expect("checked redirect URI");
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
        q.append_pair("iss", &issuer(state));
    }
    url.into()
}

async fn authorize(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let query = query.unwrap_or_default();
    let mut params: HashMap<String, String> = HashMap::new();
    let mut repeated = false;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        repeated |= params.insert(k.to_string(), v.to_string()).is_some();
    }
    let get = |k: &str| params.get(k).map(String::as_str).filter(|v| !v.is_empty());

    // Until the tool and its return address check out, nothing goes back to it.
    let Some(client_id) = get("client_id") else {
        return problem_page(&state, "client");
    };
    let client = match find_client(&state, client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return problem_page(&state, "client"),
        Err(err) => return ApiError::from(err).into_response(),
    };
    let redirect_uri = match get("redirect_uri") {
        Some(r) => r.to_string(),
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        None => return problem_page(&state, "redirect"),
    };
    if repeated || !return_matches(&client.redirect_uris, &redirect_uri) {
        return problem_page(&state, "redirect");
    }

    let tool_state = get("state").map(str::to_string);
    let fail = |code: &str, description: &str| {
        let mut pairs = vec![("error", code), ("error_description", description)];
        if let Some(s) = &tool_state {
            pairs.push(("state", s.as_str()));
        }
        Redirect::to(&back_to_tool(&state, &redirect_uri, &pairs)).into_response()
    };
    if get("response_type") != Some("code") {
        return fail(
            "unsupported_response_type",
            "only response_type=code is supported",
        );
    }
    let challenge = get("code_challenge").unwrap_or_default();
    if get("code_challenge_method") != Some("S256")
        || challenge.len() != 43
        || !challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return fail(
            "invalid_request",
            "a PKCE code_challenge with code_challenge_method=S256 is required",
        );
    }
    if tool_state.as_ref().is_some_and(|s| s.len() > 500) {
        return fail("invalid_request", "state is too long");
    }
    // The MCP spec requires `resource`; a tool that leaves it out gets the MCP address.
    let resource = match get("resource") {
        None => format!("{}/mcp", issuer(&state)),
        Some(r) => match canonical_resource(&state, r) {
            Some(r) => r,
            None => {
                return fail(
                    "invalid_target",
                    "resource must be SLAPorSLOP's MCP address",
                );
            }
        },
    };
    let scope = get("scope").map(|s| s.chars().take(500).collect::<String>());

    let id = Uuid::now_v7();
    let saved = async {
        sqlx::query(
            "insert into agent_auth_requests
                 (id, client_id, redirect_uri, state, code_challenge, resource, requested_scope, expires_at)
             values ($1, $2, $3, $4, $5, $6, $7, now() + $8)",
        )
        .bind(id)
        .bind(client.id)
        .bind(&redirect_uri)
        .bind(&tool_state)
        .bind(challenge)
        .bind(&resource)
        .bind(&scope)
        .bind(REQUEST_LIFETIME)
        .execute(&state.db)
        .await?;
        sqlx::query("update agent_clients set last_used_at = now() where id = $1")
            .bind(client.id)
            .execute(&state.db)
            .await
    };
    if let Err(err) = saved.await {
        return ApiError::from(err).into_response();
    }
    Redirect::to(&format!("{}/connect-ai/{id}", issuer(&state))).into_response()
}

// ---------------------------------------------------------------------------------------------
// The approval page

#[derive(Serialize)]
struct RequestInfo {
    /// The name the tool gives itself.
    client_name: String,
    /// `registered`: it registered itself, so its name is only its word. `document`: it's named
    /// by the address of its description, on `client_host`.
    client_kind: String,
    client_host: Option<String>,
    /// Where approving sends the browser: `loopback` (a program on this computer) or `web` (the
    /// website `redirect_host`).
    redirect_kind: &'static str,
    redirect_host: Option<String>,
    /// A website the page doesn't know: it warns before approving.
    unfamiliar: bool,
    /// The scopes the tool asked for: where the choices start.
    requested_scopes: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

type PendingRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    OffsetDateTime,
);

/// What the approval page shows: only to someone signed in, only while the request waits.
async fn request_info(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RequestInfo>> {
    if user.agent.is_some() {
        return Err(ApiError::Forbidden);
    }
    let row: Option<PendingRow> = sqlx::query_as(
        "select c.name, c.kind, c.client_id, r.redirect_uri, r.requested_scope, r.expires_at
         from agent_auth_requests r join agent_clients c on c.id = r.client_id
         where r.id = $1 and r.approved_by is null and r.expires_at > now()",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (client_name, client_kind, client_id, redirect_uri, scope, expires_at) =
        row.ok_or(ApiError::NotFound)?;
    let client_host = (client_kind == "document")
        .then(|| Url::parse(&client_id).ok()?.host_str().map(str::to_string))
        .flatten();
    let (redirect_kind, redirect_host, unfamiliar) =
        match check_return(&redirect_uri).map_err(|_| ApiError::NotFound)? {
            Return::Loopback => ("loopback", None, false),
            Return::Web(h) => {
                let known = KNOWN_WEB_HOSTS.contains(&h.as_str());
                ("web", Some(h), !known)
            }
        };
    let requested_scopes = scope
        .unwrap_or_default()
        .split_whitespace()
        .filter(|s| SCOPES.contains(s))
        .map(str::to_string)
        .collect();
    Ok(Json(RequestInfo {
        client_name,
        client_kind,
        client_host,
        redirect_kind,
        redirect_host,
        unfamiliar,
        requested_scopes,
        expires_at,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    /// What the connection is called on Account → AI agents.
    name: String,
    /// `read` or `publish`.
    scope: String,
    /// App addresses it may touch; empty: all the person's apps.
    #[serde(default)]
    apps: Vec<String>,
    #[serde(default)]
    people: bool,
    /// Days it lasts: 1 to 365.
    days: i64,
}

#[derive(Serialize)]
struct Answer {
    /// Where the browser goes next: back to the tool.
    redirect_url: String,
}

/// The signed-in person approves the tool with their choices. Cookie-authenticated, so the CSRF
/// layer applies; any signed-in session may (no "confirm it's you", SECURITY §4.3). The checks
/// are a maker token's (maker_agents.rs `create`).
async fn approve(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<Approval>,
) -> ApiResult<Json<Answer>> {
    if user.agent.is_some() {
        return Err(ApiError::Forbidden);
    }
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::invalid("name", "1 to 60 characters"));
    }
    if body.scope != "read" && body.scope != "publish" {
        return Err(ApiError::invalid("scope", "read or publish"));
    }
    if !(1..=maker_agents::MAX_DAYS).contains(&body.days) {
        return Err(ApiError::invalid(
            "days",
            format!("1 to {}", maker_agents::MAX_DAYS),
        ));
    }
    // The tool can't accept agreements, so publishing needs the Publisher Agreement accepted
    // here (the page shows it and tries again).
    if body.scope == "publish" {
        crate::legal::require(&state.db, user.id, "publisher").await?;
    }
    let mut app_ids = Vec::new();
    for slug in &body.apps {
        let app: Option<Uuid> =
            sqlx::query_scalar("select id from apps where slug = $1 and maker_id = $2")
                .bind(slug)
                .bind(user.id)
                .fetch_optional(&state.db)
                .await?;
        app_ids.push(
            app.ok_or_else(|| ApiError::invalid("apps", format!("{slug} isn't one of your apps")))?,
        );
    }
    // Checked again when the tool gets its tokens; here so the page can offer to remove one.
    if live_agents(&state.db, user.id).await? >= maker_agents::MAX_TOKENS {
        return Err(too_many());
    }
    let code = new_token();
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "update agent_auth_requests
         set approved_by = $2, code_hash = $3, name = $4, scope = $5, app_ids = $6, people = $7,
             days = $8, expires_at = now() + $9
         where id = $1 and approved_by is null and expires_at > now()
         returning redirect_uri, state",
    )
    .bind(id)
    .bind(user.id)
    .bind(hash_token(&code))
    .bind(&name)
    .bind(&body.scope)
    .bind(&app_ids)
    .bind(body.people)
    .bind(body.days as i32)
    .bind(CODE_LIFETIME)
    .fetch_optional(&state.db)
    .await?;
    let (redirect_uri, tool_state) = row.ok_or(ApiError::NotFound)?;
    let mut pairs = vec![("code", code.as_str())];
    if let Some(s) = &tool_state {
        pairs.push(("state", s.as_str()));
    }
    Ok(Json(Answer {
        redirect_url: back_to_tool(&state, &redirect_uri, &pairs),
    }))
}

/// The person says no: the request is gone, and the tool hears `access_denied`.
async fn deny(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Answer>> {
    if user.agent.is_some() {
        return Err(ApiError::Forbidden);
    }
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "delete from agent_auth_requests
         where id = $1 and approved_by is null and expires_at > now()
         returning redirect_uri, state",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (redirect_uri, tool_state) = row.ok_or(ApiError::NotFound)?;
    let mut pairs = vec![
        ("error", "access_denied"),
        ("error_description", "the person didn't allow it"),
    ];
    if let Some(s) = &tool_state {
        pairs.push(("state", s.as_str()));
    }
    Ok(Json(Answer {
        redirect_url: back_to_tool(&state, &redirect_uri, &pairs),
    }))
}

/// The account's working agent sessions (tokens and approved tools), against `MAX_TOKENS`.
async fn live_agents(db: impl sqlx::PgExecutor<'_>, user_id: Uuid) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "select count(*) from sessions
         where user_id = $1 and kind = 'agent' and revoked_at is null and expires_at > now()",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
}

fn too_many() -> ApiError {
    ApiError::Conflict(format!(
        "you have {} AI agents connected; remove one first",
        maker_agents::MAX_TOKENS
    ))
}

// ---------------------------------------------------------------------------------------------
// Tokens

#[derive(Serialize)]
struct Tokens {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    refresh_token: String,
    scope: String,
}

/// The `client_id` a request names: in the form, or as the user name of HTTP Basic (a public
/// tool may send it there, with an empty password).
fn client_id_of(headers: &HeaderMap, form: &HashMap<String, String>) -> Option<String> {
    if let Some(id) = form.get("client_id").filter(|v| !v.is_empty()) {
        return Some(id.clone());
    }
    let basic = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?;
    let decoded = String::from_utf8(STANDARD.decode(basic.trim()).ok()?).ok()?;
    let (id, _) = decoded.split_once(':')?;
    let id = url::form_urlencoded::parse(format!("x={id}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())?;
    (!id.is_empty()).then_some(id)
}

async fn token(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let result = async {
        let form = form(&body)?;
        let client_id = client_id_of(&headers, &form)
            .ok_or_else(|| oauth_error("invalid_client", "client_id is required"))?;
        match form.get("grant_type").map(String::as_str) {
            Some("authorization_code") => redeem(&state, &client_id, &form).await,
            Some("refresh_token") => refresh(&state, &client_id, &form).await,
            Some(_) => Err(oauth_error(
                "unsupported_grant_type",
                "use authorization_code or refresh_token",
            )),
            None => Err(oauth_error("invalid_request", "grant_type is required")),
        }
    }
    .await;
    match result {
        Ok(tokens) => {
            let mut res = Json(tokens).into_response();
            res.headers_mut()
                .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
            res
        }
        Err(e) => e.into_response(),
    }
}

type CodeRow = (
    Uuid,
    String,
    String,
    String,
    String,
    Uuid,
    String,
    String,
    Vec<Uuid>,
    bool,
    i32,
    Option<OffsetDateTime>,
    Option<Uuid>,
    bool,
);

/// Swaps an approved code and its PKCE verifier for tokens, making the agent session.
async fn redeem(
    state: &AppState,
    client_id: &str,
    form: &HashMap<String, String>,
) -> OAuthResult<Tokens> {
    let code = form
        .get("code")
        .ok_or_else(|| oauth_error("invalid_request", "code is required"))?;
    let verifier = form
        .get("code_verifier")
        .ok_or_else(|| oauth_error("invalid_request", "code_verifier is required (PKCE)"))?;
    let bad = || oauth_error("invalid_grant", "the code is wrong, used or expired");
    let mut tx = state.db.begin().await?;
    let row: Option<CodeRow> = sqlx::query_as(
        "select r.id, c.client_id, r.redirect_uri, r.code_challenge, r.resource, r.approved_by,
                r.name, r.scope, r.app_ids, r.people, r.days, r.redeemed_at, r.session_id,
                r.expires_at > now()
         from agent_auth_requests r join agent_clients c on c.id = r.client_id
         where r.code_hash = $1 and r.approved_by is not null
         for update of r",
    )
    .bind(hash_token(code))
    .fetch_optional(&mut *tx)
    .await?;
    let Some((
        request_id,
        expected_client,
        redirect_uri,
        challenge,
        resource,
        user_id,
        name,
        scope,
        app_ids,
        people,
        days,
        redeemed_at,
        used_session,
        live,
    )) = row
    else {
        return Err(bad());
    };
    if redeemed_at.is_some() {
        // A code used again was copied: end what its first use made (OAuth 2.1 §4.1.3).
        if let Some(session_id) = used_session {
            tracing::warn!(%session_id, "authorization code reuse; ending the connection");
            sqlx::query(
                "update sessions set revoked_at = now() where id = $1 and revoked_at is null",
            )
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        return Err(bad());
    }
    // Whatever happens next, this code is spent.
    sqlx::query("update agent_auth_requests set redeemed_at = now() where id = $1")
        .bind(request_id)
        .execute(&mut *tx)
        .await?;
    let check = (|| {
        if !live {
            return Err(bad());
        }
        if client_id != expected_client {
            return Err(oauth_error(
                "invalid_grant",
                "the code was issued to another client",
            ));
        }
        if form.get("redirect_uri").is_some_and(|r| r != &redirect_uri) {
            return Err(oauth_error(
                "invalid_grant",
                "redirect_uri doesn't match the authorization request",
            ));
        }
        if !(43..=128).contains(&verifier.len())
            || !verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
            || pkce_challenge(verifier) != challenge
        {
            return Err(oauth_error(
                "invalid_grant",
                "the code_verifier doesn't match",
            ));
        }
        if let Some(r) = form.get("resource")
            && canonical_resource(state, r).as_deref() != Some(resource.as_str())
        {
            return Err(oauth_error(
                "invalid_target",
                "resource doesn't match the authorization request",
            ));
        }
        Ok(())
    })();
    if let Err(e) = check {
        tx.commit().await?;
        return Err(e);
    }
    // Counted with the account locked, as for tokens made on the website.
    sqlx::query("select 1 from users where id = $1 for update")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    if live_agents(&mut *tx, user_id).await? >= maker_agents::MAX_TOKENS {
        tx.commit().await?;
        return Err(oauth_error(
            "invalid_grant",
            format!(
                "the account already has {} AI agents connected; remove one on SLAPorSLOP under Account → AI agents",
                maker_agents::MAX_TOKENS
            ),
        ));
    }
    let session_id = Uuid::now_v7();
    sqlx::query(
        "insert into sessions (id, user_id, kind, user_agent, expires_at)
         values ($1, $2, 'agent', $3, now() + $4)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(format!("AI agent: {name}"))
    .bind(Duration::days(i64::from(days)))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into maker_tokens (session_id, name, scope, app_ids, people) values ($1, $2, $3, $4, $5)",
    )
    .bind(session_id)
    .bind(&name)
    .bind(&scope)
    .bind(&app_ids)
    .bind(people)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "insert into agent_grants (session_id, client_id, redirect_uri, resource)
         select $1, client_id, redirect_uri, resource from agent_auth_requests where id = $2",
    )
    .bind(session_id)
    .bind(request_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("update agent_auth_requests set session_id = $2 where id = $1")
        .bind(request_id)
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("update agent_clients set last_used_at = now() where client_id = $1")
        .bind(client_id)
        .execute(&mut *tx)
        .await?;
    let tokens = issue(&mut tx, session_id, &scope, people).await?;
    tx.commit().await?;
    Ok(tokens)
}

type RefreshRow = (
    Uuid,
    Option<OffsetDateTime>,
    bool,
    Option<String>,
    String,
    String,
    bool,
);

/// Swaps a refresh token for new tokens. Each works once (OAuth 2.1 §4.3.1).
async fn refresh(
    state: &AppState,
    client_id: &str,
    form: &HashMap<String, String>,
) -> OAuthResult<Tokens> {
    let token = form
        .get("refresh_token")
        .ok_or_else(|| oauth_error("invalid_request", "refresh_token is required"))?;
    let bad = || {
        oauth_error(
            "invalid_grant",
            "the refresh token is wrong, used or expired",
        )
    };
    let mut tx = state.db.begin().await?;
    let row: Option<RefreshRow> = sqlx::query_as(
        "select r.session_id, r.used_at, coalesce(r.used_at > now() - $2, false), c.client_id, g.resource, m.scope,
                m.people
         from refresh_tokens r join sessions s on s.id = r.session_id
              join agent_grants g on g.session_id = s.id
              join maker_tokens m on m.session_id = s.id
              left join agent_clients c on c.id = g.client_id
         where r.token_hash = $1 and r.expires_at > now() and s.kind = 'agent'
         for update of r",
    )
    .bind(hash_token(token))
    .bind(REUSE_GRACE)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((session_id, used_at, just_used, owner, resource, scope, people)) = row else {
        return Err(bad());
    };
    if owner.as_deref() != Some(client_id) {
        // Another tool's refresh token: refused, and it isn't spent.
        return Err(bad());
    }
    if used_at.is_some() {
        if !just_used {
            // A used refresh token came back: it was copied. End the connection.
            tracing::warn!(%session_id, "refresh token reuse; ending the connection");
            sqlx::query(
                "update sessions set revoked_at = now() where id = $1 and revoked_at is null",
            )
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        return Err(bad());
    }
    if let Some(r) = form.get("resource")
        && canonical_resource(state, r).as_deref() != Some(resource.as_str())
    {
        return Err(oauth_error(
            "invalid_target",
            "resource doesn't match the connection",
        ));
    }
    let live: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "select exists (select 1 from sessions s join users u on u.id = s.user_id
                        where s.id = $1 and u.closed_at is null and {})",
        crate::auth::session::live_session_sql("$2", "$3")
    )))
    .bind(session_id)
    .bind(crate::auth::session::IDLE_TIMEOUT)
    .bind(crate::auth::session::DESKTOP_IDLE_TIMEOUT)
    .fetch_one(&mut *tx)
    .await?;
    if !live {
        return Err(bad());
    }
    sqlx::query("update refresh_tokens set used_at = now() where token_hash = $1")
        .bind(hash_token(token))
        .execute(&mut *tx)
        .await?;
    sqlx::query("update agent_clients set last_used_at = now() where client_id = $1")
        .bind(client_id)
        .execute(&mut *tx)
        .await?;
    let tokens = issue(&mut tx, session_id, &scope, people).await?;
    tx.commit().await?;
    Ok(tokens)
}

/// A new access token (an hour, never past the session's end) and refresh token (until the
/// session's end: the connection lasts what the person chose).
async fn issue(
    tx: &mut sqlx::PgConnection,
    session_id: Uuid,
    scope: &str,
    people: bool,
) -> sqlx::Result<Tokens> {
    let access_token = format!("{}{}", maker_agents::PREFIX, new_token());
    let refresh_token = format!("{REFRESH_PREFIX}{}", new_token());
    sqlx::query("delete from access_tokens where session_id = $1 and expires_at < now()")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    let expires_at: OffsetDateTime = sqlx::query_scalar(
        "insert into access_tokens (token_hash, session_id, expires_at)
         select $1, id, least(now() + $3, expires_at) from sessions where id = $2
         returning expires_at",
    )
    .bind(hash_token(&access_token))
    .bind(session_id)
    .bind(ACCESS_LIFETIME)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "insert into refresh_tokens (token_hash, session_id, expires_at)
         select $1, id, expires_at from sessions where id = $2",
    )
    .bind(hash_token(&refresh_token))
    .bind(session_id)
    .execute(&mut *tx)
    .await?;
    let mut granted = if scope == "publish" {
        "read publish".to_string()
    } else {
        "read".to_string()
    };
    if people {
        granted.push_str(" people");
    }
    Ok(Tokens {
        access_token,
        token_type: "Bearer",
        expires_in: (expires_at - OffsetDateTime::now_utc())
            .whole_seconds()
            .max(0),
        refresh_token,
        scope: granted,
    })
}

/// A tool ending its own connection (RFC 7009): either of its tokens ends it. Always 200, so it
/// can't be used to test tokens; another tool's token is left alone.
async fn revoke(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let result: OAuthResult<()> = async {
        let form = form(&body)?;
        let client_id = client_id_of(&headers, &form)
            .ok_or_else(|| oauth_error("invalid_client", "client_id is required"))?;
        let Some(token) = form.get("token") else {
            return Err(oauth_error("invalid_request", "token is required"));
        };
        let hash = hash_token(token);
        sqlx::query(
            "update sessions s set revoked_at = now()
             from agent_grants g join agent_clients c on c.id = g.client_id
             where g.session_id = s.id and c.client_id = $2 and s.revoked_at is null
               and s.id in (select session_id from refresh_tokens where token_hash = $1
                            union select session_id from access_tokens where token_hash = $1)",
        )
        .bind(&hash)
        .bind(&client_id)
        .execute(&state.db)
        .await?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => e.into_response(),
    }
}

// ---------------------------------------------------------------------------------------------
// Cleanup

/// Hourly (cleanup.rs): approvals that ran out, redeemed ones after a day (kept that long so a
/// code used again still ends what it made), registered tools that never started an approval
/// after a day, and tools with nothing approved still working 30 days after their last use.
pub async fn cleanup(db: &sqlx::PgPool) -> sqlx::Result<()> {
    sqlx::query(
        "delete from agent_auth_requests
         where (redeemed_at is null and expires_at < now())
            or redeemed_at < now() - interval '1 day'",
    )
    .execute(db)
    .await?;
    sqlx::query(
        "delete from agent_clients c
         where not exists (select 1 from agent_auth_requests r where r.client_id = c.id)
           and not exists (select 1 from agent_grants g join sessions s on s.id = g.session_id
                           where g.client_id = c.id and s.revoked_at is null and s.expires_at > now())
           and ((c.last_used_at is null and c.created_at < now() - interval '1 day')
                or c.last_used_at < now() - interval '30 days')",
    )
    .execute(db)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_addresses_are_https_or_this_computer() {
        assert_eq!(
            check_return("https://claude.ai/api/mcp/auth_callback"),
            Ok(Return::Web("claude.ai".into()))
        );
        assert_eq!(
            check_return("http://localhost:33418/callback"),
            Ok(Return::Loopback)
        );
        assert_eq!(check_return("http://127.0.0.1:5000/"), Ok(Return::Loopback));
        assert_eq!(check_return("http://[::1]:5000/cb"), Ok(Return::Loopback));
        for bad in [
            "http://example.com/cb",
            "http://192.168.1.2/cb",
            "javascript:alert(1)",
            "data:text/html,hi",
            "file:///etc/passwd",
            "ms-msdt:/id",
            "https://claude.ai/cb#frag",
            "https://user:pw@claude.ai/cb",
            "/relative",
            "vbscript:x",
            "cursor://anysphere.cursor-mcp/oauth/callback",
            "com.example.tool:/oauth",
        ] {
            assert!(check_return(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn loopback_returns_match_on_any_port_and_others_exactly() {
        // Claude Code's description.
        let reg = vec![
            "http://localhost/callback".to_string(),
            "https://claude.ai/api/mcp/auth_callback".to_string(),
        ];
        assert!(return_matches(&reg, "http://localhost:54321/callback"));
        assert!(return_matches(&reg, "http://127.0.0.1:54321/callback"));
        assert!(!return_matches(&reg, "http://localhost:54321/other"));
        assert!(!return_matches(&reg, "http://localhost:54321/callback?x=1"));
        assert!(!return_matches(&reg, "https://localhost:54321/callback"));
        assert!(return_matches(
            &reg,
            "https://claude.ai/api/mcp/auth_callback"
        ));
        assert!(!return_matches(
            &reg,
            "https://claude.ai/api/mcp/auth_callback/"
        ));
        assert!(!return_matches(
            &reg,
            "https://claude.ai:444/api/mcp/auth_callback"
        ));
        assert!(!return_matches(
            &reg,
            "https://evil.example/api/mcp/auth_callback"
        ));
    }

    #[test]
    fn names_are_tidied() {
        assert_eq!(tidy_name("  Claude\u{202E}  Code \n"), "Claude Code");
        assert_eq!(tidy_name(""), "AI tool");
        assert_eq!(tidy_name(&"x".repeat(100)).chars().count(), 60);
    }

    #[test]
    fn description_addresses_follow_the_draft() {
        assert!(
            check_document_address("https://claude.ai/oauth/claude-code-client-metadata", false)
                .is_some()
        );
        for bad in [
            "https://claude.ai/",
            "https://claude.ai",
            "http://claude.ai/x",
            "https://claude.ai/x#y",
            "https://claude.ai/x?y=1",
            "https://u@claude.ai/x",
            "https://claude.ai/a/../x",
            "https://127.0.0.1/x",
            "https://CLAUDE.ai/x",
            "slopcl_abc",
        ] {
            assert!(check_document_address(bad, false).is_none(), "{bad}");
        }
        assert!(check_document_address("http://127.0.0.1:9/x.json", true).is_some());
    }

    #[test]
    fn descriptions_must_name_themselves_and_use_no_secret() {
        let addr = "https://tool.example/client.json";
        let good = json!({ "client_id": addr, "client_name": "Tool", "redirect_uris": ["http://127.0.0.1:3000/cb"] });
        assert_eq!(
            check_document(&good, addr),
            Some(("Tool".into(), vec!["http://127.0.0.1:3000/cb".into()]))
        );
        let mut other = good.clone();
        other["client_id"] = json!("https://evil.example/client.json");
        assert!(check_document(&other, addr).is_none());
        let mut secret = good.clone();
        secret["token_endpoint_auth_method"] = json!("client_secret_basic");
        assert!(check_document(&secret, addr).is_none());
        let mut bad_uri = good.clone();
        bad_uri["redirect_uris"] = json!(["http://evil.example/cb"]);
        assert!(check_document(&bad_uri, addr).is_none());
        let mut nameless = good;
        nameless.as_object_mut().unwrap().remove("client_name");
        assert_eq!(check_document(&nameless, addr).unwrap().0, "tool.example");
    }

    #[test]
    fn cache_headers_set_how_long_a_description_is_kept() {
        let mut h = HeaderMap::new();
        assert_eq!(cache_age(&h), DOCUMENT_DEFAULT_AGE);
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=600"),
        );
        assert_eq!(cache_age(&h), StdDuration::from_secs(600));
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=99999999"),
        );
        assert_eq!(cache_age(&h), DOCUMENT_MAX_AGE);
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        assert_eq!(cache_age(&h), DOCUMENT_MIN_AGE);
    }

    #[test]
    fn registrations_are_counted_per_address_and_ipv6_network() {
        let r = Registrations::default();
        let now = Instant::now();
        let a: IpAddr = "2001:db8::1".parse().unwrap();
        let b: IpAddr = "2001:db8::2".parse().unwrap();
        for _ in 0..REGISTRATIONS_PER_HOUR / 2 {
            assert!(r.take(a, now));
            assert!(r.take(b, now));
        }
        assert!(!r.take(a, now));
        assert!(r.take("2001:db8:1::1".parse().unwrap(), now));
        assert!(r.take(a, now + StdDuration::from_secs(3601)));
    }
}
