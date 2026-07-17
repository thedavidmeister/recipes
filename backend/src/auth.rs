//! Telegram magic-link auth (#25). **Mandatory**: every API endpoint requires a
//! session, so this module is the front door to the whole corpus.
//!
//! Auth exists because #20 needs identity — a group deciding what to cook is a
//! headcount, and "everyone said yes" is meaningless without knowing who
//! everyone is. It is *not* what protects the corpus: nothing writes it from
//! outside, and ingest refuses a host no adapter claims before fetching, so the
//! surface underneath is already safe by construction. Auth is additive.
//!
//! ## The bot logs you in; the site only points at it
//!
//! 1. The site shows a link to the bot. Pressing **Start** sends it `/start`
//!    along with your Telegram id — that id is the login.
//! 2. The bot replies **to you** with a one-time completion link.
//! 3. Opening it sets the session cookie **in your browser**.
//!
//! ## Why it runs in that direction — the part that is easy to get wrong
//!
//! The intuitive flow is the reverse: the browser starts a login, shows a link,
//! and waits for someone to tap it. That hands the capability to *redeem* to
//! whoever *started* the login, while the identity comes from whoever *tapped* —
//! and nothing ties those to the same person. So an attacker starts a login,
//! sends the link to a victim, and redeems a session **as the victim** the moment
//! they tap it.
//!
//! That is not hypothetical. It was built here, defended in comments, and then
//! demonstrated end-to-end as a full account takeover, which is why this design
//! replaced it. Splitting the link's nonce from a separate poll secret does not
//! help — it defends "someone else saw my link", not "the person who sent me this
//! link is the attacker".
//!
//! So the secret that redeems a session is minted **for** a Telegram user we have
//! been told about, and delivered **to** their private chat. Whoever holds it is
//! whoever the bot sent it to, and there is nothing to hand a victim. **Do not
//! add a browser-initiated login flow** without re-deriving this first.
//!
//! The cost is real and accepted: the session lands in whichever browser opens
//! the bot's link, so signing in a desktop from Telegram on a phone does not
//! work. Cross-device session transfer *is* the attack above. A code typed into
//! the waiting device would restore it, at the price of a code that can be talked
//! out of someone.
//!
//! A bot cannot message someone who has not contacted it first
//! (`Forbidden: bot can't initiate conversation with a user`), which is why the
//! user messages the bot rather than us messaging them — and why the
//! email-bombing vector of a classic magic link does not exist here.

use axum::{
    extract::{Request, State},
    http::{
        header::{COOKIE, SET_COOKIE},
        HeaderMap, StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use libsql::Connection;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{error::AppError, AppState};

/// How long a completion link stays usable. Short: until redeemed or expired it
/// is a live credential sitting in a chat message.
const COMPLETION_TTL_SECS: i64 = 15 * 60;

/// How long a session lasts. Long enough not to ask a group to re-authenticate
/// mid-dinner.
const SESSION_TTL_SECS: i64 = 30 * 24 * 60 * 60;

const SECRET_BYTES: usize = 32;

/// Name of the session cookie.
const SESSION_COOKIE: &str = "recipes_session";

/// Backend-only Telegram config. Absent config is a startup error rather than a
/// per-request surprise: with mandatory auth, a backend that cannot mint a login
/// can serve nothing.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot API token. A backend secret, handled like the Turso write token — it
    /// never reaches the browser.
    pub bot_token: String,
    /// Shared secret Telegram echoes back in `X-Telegram-Bot-Api-Secret-Token`.
    /// Without it the webhook is a public endpoint anyone can POST a forged
    /// `/start` to, claiming any Telegram id — i.e. logging in as anyone. See
    /// [`verify_webhook_origin`].
    pub webhook_secret: String,
    /// Where the completion link points, e.g. `https://recipes.lehlehleh.com`.
    /// The bot sends an absolute URL, so the backend has to know the site.
    pub frontend_base_url: String,
}

impl TelegramConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            bot_token: req_env("TELEGRAM_BOT_TOKEN")?,
            webhook_secret: req_env("TELEGRAM_WEBHOOK_SECRET")?,
            frontend_base_url: req_env("FRONTEND_BASE_URL")?
                .trim_end_matches('/')
                .to_owned(),
        })
    }
}

fn req_env(key: &str) -> anyhow::Result<String> {
    let v = std::env::var(key)
        .map_err(|_| anyhow::anyhow!("{key} is required (auth is mandatory — see #25)"))?;
    if v.trim().is_empty() {
        anyhow::bail!("{key} is set but empty");
    }
    Ok(v)
}

/// The key that guards `/api/ingest`.
///
/// Ingest is a server-driven corpus sync (#49) triggered by a schedule, not a
/// person — so it authenticates a **machine**, not a session. Like the webhook
/// secret this is a shared secret we mint and hold in the environment; unlike it,
/// the convention is ours, so we use the standard `Authorization: Bearer` rather
/// than a bespoke `X-` header (the `X-` prefix is deprecated for new headers, and
/// proxies and log pipelines redact `Authorization` by convention).
pub fn ingest_key_from_env() -> anyhow::Result<String> {
    req_env("INGEST_API_KEY")
}

/// How the session cookie is scoped.
#[derive(Debug, Clone)]
pub struct CookieConfig {
    /// Parent domain both services live under, e.g. `lehlehleh.com`, so
    /// `recipes.` and `api.recipes.` share one cookie. `None` scopes it to the
    /// host that set it.
    ///
    /// This is what `onrender.com` could not do: it is on the Public Suffix List,
    /// so browsers reject a `Domain` of it — two Render subdomains are different
    /// *sites*. Owning the domain is what makes a shared cookie legal, and a
    /// shared cookie is what reaches #20's WebSocket.
    pub domain: Option<String>,
    /// Whether the cookie is `Secure`. Configured **explicitly**, never inferred
    /// from another setting: the failure is silent and one-directional — nobody
    /// notices a missing `Secure` until the session is already on the wire.
    pub secure: bool,
}

impl CookieConfig {
    /// Reads the environment; [`CookieConfig::parse`] holds the rules.
    ///
    /// Env is touched only here so the rules stay testable without mutating
    /// process-global state: `std::env::set_var` is unsound under a threaded
    /// test runner (it is `unsafe` in edition 2024), and two tests racing on one
    /// variable surfaces as an unrelated crash rather than a failed assert.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::parse(
            std::env::var("COOKIE_SECURE").ok().as_deref(),
            std::env::var("COOKIE_DOMAIN").ok().as_deref(),
        )
    }

    /// Fails closed: `COOKIE_SECURE` must be stated, and the only route to a
    /// non-`Secure` cookie is asking for one.
    fn parse(secure: Option<&str>, domain: Option<&str>) -> anyhow::Result<Self> {
        let secure = match secure {
            Some("true") => true,
            Some("false") => false,
            Some(other) => anyhow::bail!("COOKIE_SECURE must be `true` or `false`, got `{other}`"),
            None => anyhow::bail!(
                "COOKIE_SECURE is required: `true` anywhere reachable, `false` only for local http"
            ),
        };
        Ok(Self {
            domain: domain.filter(|d| !d.trim().is_empty()).map(str::to_owned),
            secure,
        })
    }
}

/// A secret we mint and hand out exactly once.
fn mint_secret() -> String {
    let mut buf = [0u8; SECRET_BYTES];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Hash a secret for storage and lookup.
///
/// SHA-256 with no salt or KDF is right *here specifically*: these are 256 bits
/// of CSPRNG output, not passwords. There is no dictionary to attack and nothing
/// to guess, so a KDF would tax every request against an attack that cannot
/// happen. What it does defend is the case that matters — a leaked database
/// yields no usable credential.
///
/// Hashing also makes the digest the lookup key, so a secret is never compared in
/// application code and there is no timing signal to leak.
fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
pub struct CompleteRequest {
    /// The secret from the bot's link.
    pub c: String,
}

#[derive(Debug, Serialize)]
pub struct CompleteResponse {
    pub username: Option<String>,
}

/// `POST /api/auth/complete` — redeem the bot's link for a session.
///
/// Public, because it is *how* a session is obtained; the secret is the
/// authentication. Single-use: the row is deleted as it is redeemed, so a
/// replayed link gets nothing. The token is returned **only** as an `HttpOnly`
/// cookie and stored only as a hash, so it exists in two places the client cannot
/// read: the browser's cookie jar, and a digest in our database.
pub async fn complete(
    State(state): State<AppState>,
    Json(req): Json<CompleteRequest>,
) -> Result<Response, AppError> {
    let hash = hash_secret(&req.c);

    let mut rows = state
        .db
        .query(
            "SELECT telegram_user_id, username, expires_at
             FROM login_completions WHERE completion_hash = ?1",
            libsql::params![hash.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("login lookup failed: {e}")))?;

    // An unknown secret and an expired one get the same answer on purpose: a
    // caller guessing secrets learns nothing about which ones ever existed.
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("login lookup failed: {e}")))?
    else {
        return Err(AppError::Unauthorized(
            "that link is expired or used".into(),
        ));
    };

    let telegram_user_id: String = row.get(0).map_err(row_err)?;
    let username: Option<String> = row.get(1).map_err(row_err)?;
    let expires_at: i64 = row.get(2).map_err(row_err)?;

    // Burn it first: redeemed or expired, it must not survive this call.
    delete_completion(&state.db, &hash).await?;

    // Checked here, not left to a sweep: a sweep that has not run yet must never
    // mean an expired credential still works.
    if now() >= expires_at {
        return Err(AppError::Unauthorized(
            "that link is expired or used".into(),
        ));
    }

    let user_id = upsert_user(&state.db, &telegram_user_id, username.as_deref()).await?;
    let token = issue_session(&state.db, user_id).await?;

    let mut res = Json(CompleteResponse { username }).into_response();
    res.headers_mut().insert(
        SET_COOKIE,
        session_cookie(&token, &state.cookie)
            .parse()
            .map_err(|e| AppError::Internal(format!("bad cookie: {e}")))?,
    );
    Ok(res)
}

/// Build the session cookie.
///
/// `HttpOnly` is the point of the transport: script cannot read it, so an XSS can
/// ride the session but cannot exfiltrate it for later or offline reuse — which a
/// token in JS-reachable storage cannot prevent.
///
/// `SameSite=Lax` suffices rather than `None` **because we own the domain**:
/// `recipes.lehlehleh.com` and `api.recipes.lehlehleh.com` share a registrable
/// domain, so traffic between them is same-site and the cookie rides along —
/// including #20's WebSocket handshake, which is cross-origin but same-site.
/// `SameSite=None` would make it a third-party cookie, which Safari and Firefox
/// block by default.
fn session_cookie(token: &str, cfg: &CookieConfig) -> String {
    let mut c = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={SESSION_TTL_SECS}"
    );
    if let Some(domain) = &cfg.domain {
        c.push_str(&format!("; Domain={domain}"));
    }
    if cfg.secure {
        c.push_str("; Secure");
    }
    c
}

fn row_err(e: libsql::Error) -> AppError {
    AppError::Internal(format!("row decode failed: {e}"))
}

async fn delete_completion(conn: &Connection, hash: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM login_completions WHERE completion_hash = ?1",
        libsql::params![hash.to_owned()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("could not clear login: {e}")))?;
    Ok(())
}

/// Find or create the user behind a Telegram id, tracking their display name.
///
/// Keyed on the numeric id, never the username — a username can be released and
/// claimed by someone else, so a username-keyed account could be inherited.
///
/// The username is overwritten with whatever Telegram last reported, **including
/// nothing**: a handle that has been removed must not linger, because it may now
/// belong to a different person and would name the wrong one.
async fn upsert_user(
    conn: &Connection,
    telegram_user_id: &str,
    username: Option<&str>,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO users (telegram_user_id, username) VALUES (?1, ?2)
         ON CONFLICT(telegram_user_id) DO UPDATE SET username = excluded.username",
        libsql::params![telegram_user_id.to_owned(), username.map(str::to_owned)],
    )
    .await
    .map_err(|e| AppError::Internal(format!("could not upsert user: {e}")))?;

    let mut rows = conn
        .query(
            "SELECT id FROM users WHERE telegram_user_id = ?1",
            libsql::params![telegram_user_id.to_owned()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("could not read user: {e}")))?;
    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("could not read user: {e}")))?
        .ok_or_else(|| AppError::Internal("user vanished after upsert".into()))?;
    row.get::<i64>(0).map_err(row_err)
}

async fn issue_session(conn: &Connection, user_id: i64) -> Result<String, AppError> {
    let token = mint_secret();
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
        libsql::params![hash_secret(&token), user_id, now() + SESSION_TTL_SECS],
    )
    .await
    .map_err(|e| AppError::Internal(format!("could not issue session: {e}")))?;
    Ok(token)
}

/// The authenticated user, attached to a request by [`require_session`].
#[derive(Debug, Clone)]
pub struct CurrentUser {
    #[allow(dead_code)] // #20 is what reads this.
    pub id: i64,
    pub telegram_user_id: String,
    pub username: Option<String>,
}

/// Reject any request without a live session.
///
/// Auth is mandatory (#25), so this wraps everything except `/health` and the
/// login endpoints themselves. Since #29 the client drives ingestion and the
/// server performs it, so `/api/ingest` **is what a search does** — gating it
/// gates search, deliberately.
pub async fn require_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = session_from_cookie(req.headers())
        .ok_or_else(|| AppError::Unauthorized("a session is required".into()))?;

    let mut rows = state
        .db
        .query(
            "SELECT s.user_id, s.expires_at, u.telegram_user_id, u.username
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?1",
            libsql::params![hash_secret(&token)],
        )
        .await
        .map_err(|e| AppError::Internal(format!("session lookup failed: {e}")))?;

    let row = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("session lookup failed: {e}")))?
        .ok_or_else(|| AppError::Unauthorized("unknown or expired session".into()))?;

    let user_id: i64 = row.get(0).map_err(row_err)?;
    let expires_at: i64 = row.get(1).map_err(row_err)?;
    let telegram_user_id: String = row.get(2).map_err(row_err)?;
    let username: Option<String> = row.get(3).map_err(row_err)?;

    // Checked on read, not left to a sweep: an expired session must be dead the
    // moment it expires, whatever housekeeping has or has not run.
    if now() >= expires_at {
        return Err(AppError::Unauthorized("unknown or expired session".into()));
    }

    req.extensions_mut().insert(CurrentUser {
        id: user_id,
        telegram_user_id,
        username,
    });
    Ok(next.run(req).await)
}

/// `GET /api/me` — who am I?
///
/// The SPA cannot answer this itself: the session is an `HttpOnly` cookie, so
/// script cannot see whether one exists, let alone whose. That is the point of
/// the cookie, and it is why this endpoint has to exist.
///
/// Guarded like everything else, so 401 *is* the answer "not logged in". It is
/// also how the tab that showed the bot link notices the login: when the
/// completion link is opened in the same browser, the cookie appears and this
/// starts answering.
pub async fn me(Extension(user): Extension<CurrentUser>) -> Json<MeResponse> {
    Json(MeResponse {
        telegram_user_id: user.telegram_user_id,
        username: user.username,
    })
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub telegram_user_id: String,
    pub username: Option<String>,
}

/// `POST /api/auth/logout` — drop the session.
///
/// Deletes the row *and* expires the cookie. Deleting the row is what ends the
/// session — the gate looks the token up, so a cookie the client kept would be
/// useless anyway. Clearing it only saves the browser sending a corpse.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(token) = session_from_cookie(&headers) {
        state
            .db
            .execute(
                "DELETE FROM sessions WHERE token_hash = ?1",
                libsql::params![hash_secret(&token)],
            )
            .await
            .map_err(|e| AppError::Internal(format!("logout failed: {e}")))?;
    }

    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut().insert(
        SET_COOKIE,
        expire_cookie(&state.cookie)
            .parse()
            .map_err(|e| AppError::Internal(format!("bad cookie: {e}")))?,
    );
    Ok(res)
}

/// A cookie telling the browser to forget the session. Attributes must match the
/// ones it was set with, or the browser keeps the original alongside it.
fn expire_cookie(cfg: &CookieConfig) -> String {
    let mut c = format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0");
    if let Some(domain) = &cfg.domain {
        c.push_str(&format!("; Domain={domain}"));
    }
    if cfg.secure {
        c.push_str("; Secure");
    }
    c
}

/// Pull the session out of the `Cookie` header.
///
/// Hand-parsed rather than pulling in a cookie crate: this reads one name from a
/// header, and the parse is a security boundary, so it is worth being able to see
/// all of it. A `Cookie` header is `a=1; b=2`, and a name must match whole —
/// matching a prefix would let `xsession=…` satisfy `session=…`.
fn session_from_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == SESSION_COOKIE)
            .then(|| value.trim())
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    })
}

/// A Telegram `Update`, narrowed to the part that logs someone in.
#[derive(Debug, Deserialize)]
pub struct Update {
    pub message: Option<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub text: Option<String>,
    pub from: Option<TelegramUser>,
    pub chat: Option<Chat>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
    pub id: i64,
}

/// Reject any request to ingest without the infra API key.
///
/// `/api/ingest` is machine-only: a schedule triggers the corpus sync, and no
/// browser ever calls it (#49). So it is gated by a key rather than a session —
/// deliberately a *different principal*, not a skeleton key: this authenticates
/// "our infrastructure", never a user, and it grants nothing but the sync.
///
/// A session cookie does not open this door, which is the point — the frontend
/// has no access to ingestion at all.
pub async fn require_api_key(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    if !verify_bearer(req.headers(), &state.ingest_key) {
        tracing::warn!("rejected an ingest call with a missing or bad api key");
        return Err(AppError::Unauthorized("a valid api key is required".into()));
    }
    Ok(next.run(req).await)
}

/// Does this request carry our infra key?
///
/// Standard `Authorization: Bearer <key>`. Compared in constant time for the same
/// reason as [`verify_webhook_origin`]: two secrets meet in application code here,
/// so a byte-wise early exit would leak the prefix.
fn verify_bearer(headers: &HeaderMap, expected: &str) -> bool {
    let Some(got) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    got.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Is this request really from Telegram?
///
/// The webhook URL is public, so without this anyone could POST a handcrafted
/// `/start` naming any `telegram_user_id` — forging a login for an account they
/// do not own. Telegram echoes the secret registered with `setWebhook` in
/// `X-Telegram-Bot-Api-Secret-Token`.
///
/// Compared in constant time: this *is* a case where two secrets meet in
/// application code, so a byte-wise early exit would leak the prefix.
fn verify_webhook_origin(headers: &HeaderMap, expected: &str) -> bool {
    let Some(got) = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    got.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// `POST /api/telegram/webhook` — someone pressed Start.
///
/// This is where a login begins, and the direction matters: Telegram tells us
/// *which user* messaged the bot, we mint a completion secret **for that user**,
/// and the bot sends it to **their** private chat. There is no way to hand it to
/// anyone else, which is the whole design (see the module docs).
///
/// Answers 200 for anything it will not act on: Telegram retries non-2xx, so
/// erroring on a message we do not care about earns a retry storm. A forged
/// origin is the exception — 401, no work.
pub async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<Update>,
) -> Result<StatusCode, AppError> {
    if !verify_webhook_origin(&headers, &state.telegram.webhook_secret) {
        tracing::warn!("rejected a webhook call with a bad secret token");
        return Err(AppError::Unauthorized("bad webhook secret".into()));
    }

    let Some(message) = update.message else {
        return Ok(StatusCode::OK);
    };
    let (Some(text), Some(from), Some(chat)) = (message.text, message.from, message.chat) else {
        return Ok(StatusCode::OK);
    };

    // Telegram sends a bare `/start` when the deep link carries no payload, which
    // is the normal case here: the link exists to open a chat, not to carry state.
    // A payload would be ignored — there is nothing a caller could usefully say.
    if !text.trim_start().starts_with("/start") {
        reply(&state, chat.id, "Send /start to sign in.").await;
        return Ok(StatusCode::OK);
    }

    let link = mint_completion(&state, &from).await?;
    reply(
        &state,
        chat.id,
        &format!(
            "Tap to finish signing in. This link is yours alone and lasts 15 minutes:\n{link}"
        ),
    )
    .await;
    Ok(StatusCode::OK)
}

/// Mint a completion secret for a **known** Telegram user and build their link.
///
/// The user is not something a caller chose: it arrived from Telegram alongside a
/// verified webhook secret.
async fn mint_completion(state: &AppState, user: &TelegramUser) -> Result<String, AppError> {
    let secret = mint_secret();
    state
        .db
        .execute(
            "INSERT INTO login_completions (completion_hash, telegram_user_id, username, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            libsql::params![
                hash_secret(&secret),
                user.id.to_string(),
                user.username.clone(),
                now() + COMPLETION_TTL_SECS
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("could not mint login: {e}")))?;

    // Cheap, and it is why this table cannot grow without bound: a login is the
    // only thing that writes it, and each one clears the dead rows behind it. No
    // scheduler, and no anonymous caller who could outrun it — reaching here at
    // all costs an authenticated Telegram round trip.
    if let Err(e) = sweep_expired(&state.db).await {
        tracing::warn!("could not sweep expired auth rows: {e}");
    }

    Ok(format!(
        "{}/auth/finish?c={secret}",
        state.telegram.frontend_base_url
    ))
}

/// Best-effort reply. A failed courtesy message must not fail a login already
/// minted, so this logs and moves on.
async fn reply(state: &AppState, chat_id: i64, text: &str) {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        state.telegram.bot_token
    );
    // Deliberately not the SSRF-guarded client: that exists to fetch
    // attacker-influenced URLs, and this is a fixed first-party endpoint.
    if let Err(e) = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true
        }))
        .send()
        .await
    {
        tracing::warn!("could not reply to chat {chat_id}: {e}");
    }
}

/// Delete expired logins and sessions. Housekeeping only — both are refused on
/// read, so this reclaims rows rather than enforcing anything.
pub async fn sweep_expired(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM login_completions WHERE expires_at <= unixepoch()",
        (),
    )
    .await?;
    conn.execute("DELETE FROM sessions WHERE expires_at <= unixepoch()", ())
        .await?;
    Ok(())
}

/// Mint a real session for a Telegram id, for tests that need to be *past* the
/// gate. Goes through the same code the login path does, so a test cannot prove
/// the gate against a session the real flow could not issue.
#[cfg(test)]
pub async fn issue_test_session(conn: &Connection, telegram_user_id: &str) -> String {
    let user = upsert_user(conn, telegram_user_id, None).await.unwrap();
    issue_session(conn, user).await.unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    #[test]
    fn minted_secrets_are_random_and_url_safe() {
        let a = mint_secret();
        let b = mint_secret();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn secrets_are_hashed_at_rest() {
        let secret = mint_secret();
        let hashed = hash_secret(&secret);
        assert_ne!(hashed, secret);
        assert_eq!(hashed, hash_secret(&secret), "hashing must be stable");
        assert_ne!(hashed, hash_secret(&mint_secret()));
    }

    /// Without this the webhook is a public endpoint anyone can POST a forged
    /// `/start` to, claiming any Telegram id — i.e. forging a login as anyone.
    #[test]
    fn webhook_origin_is_verified() {
        let mut headers = HeaderMap::new();
        assert!(!verify_webhook_origin(&headers, "s3cret"), "no header");

        headers.insert(
            "x-telegram-bot-api-secret-token",
            HeaderValue::from_static("wrong"),
        );
        assert!(!verify_webhook_origin(&headers, "s3cret"), "wrong secret");

        // A prefix must not pass — the compare is whole-value.
        headers.insert(
            "x-telegram-bot-api-secret-token",
            HeaderValue::from_static("s3cre"),
        );
        assert!(!verify_webhook_origin(&headers, "s3cret"), "prefix");

        headers.insert(
            "x-telegram-bot-api-secret-token",
            HeaderValue::from_static("s3cret"),
        );
        assert!(verify_webhook_origin(&headers, "s3cret"));
    }

    #[test]
    fn session_cookie_parsing() {
        let mut h = HeaderMap::new();
        assert_eq!(session_from_cookie(&h), None, "no Cookie header");

        h.insert(COOKIE, HeaderValue::from_static("recipes_session=abc"));
        assert_eq!(session_from_cookie(&h).as_deref(), Some("abc"));

        h.insert(
            COOKIE,
            HeaderValue::from_static("theme=dark; recipes_session=abc; lang=en"),
        );
        assert_eq!(session_from_cookie(&h).as_deref(), Some("abc"));

        // A name must match WHOLE — else an attacker-set `xrecipes_session` could
        // stand in for the real one.
        for hostile in [
            "xrecipes_session=abc",
            "recipes_session_x=abc",
            "notrecipes_session=abc",
        ] {
            h.insert(COOKIE, HeaderValue::from_str(hostile).unwrap());
            assert_eq!(session_from_cookie(&h), None, "{hostile} must not match");
        }

        h.insert(COOKIE, HeaderValue::from_static("recipes_session="));
        assert_eq!(session_from_cookie(&h), None, "empty is not a session");

        h.insert(COOKIE, HeaderValue::from_static("garbage"));
        assert_eq!(session_from_cookie(&h), None);
    }

    /// The reason a cookie was chosen over a bearer token: script must not read
    /// the session.
    #[test]
    fn the_session_cookie_is_httponly_and_lax() {
        let prod = CookieConfig {
            domain: Some("lehlehleh.com".into()),
            secure: true,
        };
        let c = session_cookie("tok123", &prod);
        assert!(c.contains("HttpOnly"), "script must not read it: {c}");
        assert!(c.contains("SameSite=Lax"), "{c}");
        assert!(c.contains("Secure"), "{c}");
        // Scoped to the parent domain so `recipes.` and `api.recipes.` share it —
        // legal only because we own the domain. `onrender.com` is on the Public
        // Suffix List, so a Domain of it is rejected outright.
        assert!(c.contains("Domain=lehlehleh.com"), "{c}");
        assert!(c.starts_with("recipes_session=tok123;"), "{c}");
    }

    #[test]
    fn the_dev_cookie_is_host_scoped_and_not_secure() {
        let dev = CookieConfig {
            domain: None,
            secure: false,
        };
        let c = session_cookie("tok123", &dev);
        assert!(
            c.contains("HttpOnly"),
            "HttpOnly is not a prod-only luxury: {c}"
        );
        assert!(!c.contains("Secure"), "{c}");
        assert!(!c.contains("Domain="), "{c}");
    }

    #[test]
    fn a_minted_cookie_round_trips_through_the_gate_parser() {
        let token = mint_secret();
        let set = session_cookie(
            &token,
            &CookieConfig {
                domain: Some("lehlehleh.com".into()),
                secure: true,
            },
        );
        let pair = set.split(';').next().unwrap().to_owned();
        let mut h = HeaderMap::new();
        h.insert(COOKIE, HeaderValue::from_str(&pair).unwrap());
        assert_eq!(session_from_cookie(&h).as_deref(), Some(token.as_str()));
    }

    /// `Secure` must be stated, never inferred: a forgotten variable that
    /// silently puts a session on plain http is not a failure anyone notices.
    #[test]
    fn cookie_config_fails_closed_on_missing_secure() {
        assert!(
            CookieConfig::parse(None, None).is_err(),
            "unset COOKIE_SECURE must refuse to start"
        );
        for bad in ["yes", "1", "TRUE", "", "  "] {
            assert!(
                CookieConfig::parse(Some(bad), None).is_err(),
                "{bad:?} must refuse to start rather than read as false"
            );
        }
        assert!(CookieConfig::parse(Some("true"), None).unwrap().secure);
        assert!(!CookieConfig::parse(Some("false"), None).unwrap().secure);
    }

    /// A blank `COOKIE_DOMAIN` means unset, not a cookie scoped to "".
    #[test]
    fn a_blank_cookie_domain_is_no_domain() {
        assert_eq!(
            CookieConfig::parse(Some("true"), Some("  "))
                .unwrap()
                .domain,
            None
        );
        assert_eq!(
            CookieConfig::parse(Some("true"), None).unwrap().domain,
            None
        );
        assert_eq!(
            CookieConfig::parse(Some("true"), Some("lehlehleh.com"))
                .unwrap()
                .domain
                .as_deref(),
            Some("lehlehleh.com")
        );
    }

    /// Identity is the numeric id: a user who renames stays one account.
    #[tokio::test]
    async fn a_renamed_user_is_still_the_same_account() {
        let conn = conn().await;
        let first = upsert_user(&conn, "4242", Some("dave")).await.unwrap();
        let second = upsert_user(&conn, "4242", Some("david")).await.unwrap();
        assert_eq!(first, second);

        let mut rows = conn.query("SELECT count(*) FROM users", ()).await.unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1
        );

        let mut rows = conn
            .query(
                "SELECT username FROM users WHERE id = ?1",
                libsql::params![first],
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            "david"
        );
    }

    /// A removed handle must not linger: it may now belong to someone else, and
    /// showing it would name the wrong person.
    #[tokio::test]
    async fn a_removed_username_is_cleared_not_kept() {
        let conn = conn().await;
        let id = upsert_user(&conn, "4242", Some("dave")).await.unwrap();
        upsert_user(&conn, "4242", None).await.unwrap();

        let mut rows = conn
            .query(
                "SELECT username FROM users WHERE id = ?1",
                libsql::params![id],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert!(
            row.get::<Option<String>>(0).unwrap().is_none(),
            "a handle Telegram no longer reports must not survive"
        );
    }

    #[tokio::test]
    async fn distinct_telegram_ids_are_distinct_users() {
        let conn = conn().await;
        let a = upsert_user(&conn, "1", Some("dave")).await.unwrap();
        let b = upsert_user(&conn, "2", Some("dave")).await.unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn a_session_token_is_stored_only_as_a_hash() {
        let conn = conn().await;
        let user = upsert_user(&conn, "4242", None).await.unwrap();
        let token = issue_session(&conn, user).await.unwrap();

        let mut rows = conn
            .query("SELECT token_hash FROM sessions", ())
            .await
            .unwrap();
        let stored: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_ne!(stored, token, "the raw token must never be stored");
        assert_eq!(stored, hash_secret(&token));
    }

    #[tokio::test]
    async fn sweep_clears_expired_rows_only() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO login_completions (completion_hash, telegram_user_id, expires_at)
             VALUES ('fresh', '1', unixepoch() + 900), ('stale', '1', unixepoch() - 1)",
            (),
        )
        .await
        .unwrap();

        let user = upsert_user(&conn, "1", None).await.unwrap();
        conn.execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ('dead', ?1, unixepoch() - 1)",
            libsql::params![user],
        )
        .await
        .unwrap();
        let live = issue_session(&conn, user).await.unwrap();

        sweep_expired(&conn).await.unwrap();

        let mut rows = conn
            .query("SELECT completion_hash FROM login_completions", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            "fresh"
        );

        let mut rows = conn
            .query(
                "SELECT count(*) FROM sessions WHERE token_hash = ?1",
                libsql::params![hash_secret(&live)],
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1,
            "a live session must survive the sweep"
        );
    }
}
