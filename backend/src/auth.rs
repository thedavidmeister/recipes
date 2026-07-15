//! Telegram magic-link auth (#25). **Mandatory**: every API endpoint requires a
//! session, so this module is the front door to the whole corpus.
//!
//! Auth exists because #20 needs identity — a group deciding what to cook is a
//! headcount, and "everyone said yes" is meaningless without knowing who
//! everyone is. It is *not* what protects the corpus: nothing writes it from
//! outside, and ingest refuses a host no adapter claims before fetching, so the
//! surface underneath was already safe by construction. Auth is additive.
//!
//! ## The link goes TO the bot, not from it
//!
//! The intuitive design — "give us your username, we'll DM you a link" — is
//! impossible: `Forbidden: bot can't initiate conversation with a user` is a hard
//! Telegram restriction. So the magic link inverts. We show a
//! `t.me/<bot>?start=<nonce>` link; tapping it opens Telegram, and pressing Start
//! sends the bot `/start <nonce>` **together with the user's Telegram id**. That
//! id is the login.
//!
//! A useful side effect: a bot cannot message someone who never started it, so
//! the nonce is only ever delivered to a user who opted in by tapping — the
//! email-bombing vector of a classic magic link simply does not exist here.

use axum::{
    extract::{Request, State},
    http::{
        header::{COOKIE, SET_COOKIE},
        HeaderMap, StatusCode,
    },
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use libsql::Connection;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{error::AppError, AppState};

/// How long an unclaimed login attempt stays usable. Short, because until it is
/// claimed or expires it is a live credential sitting in a link.
const NONCE_TTL_SECS: i64 = 15 * 60;

/// How long a session lasts. Long enough that a cooking app does not ask a group
/// to re-authenticate mid-dinner.
const SESSION_TTL_SECS: i64 = 30 * 24 * 60 * 60;

/// Telegram caps a `start` payload at 64 chars from `[A-Za-z0-9_-]`, so 32 random
/// bytes hex-encoded lands exactly on the limit.
const SECRET_BYTES: usize = 32;

/// Name of the session cookie.
const SESSION_COOKIE: &str = "recipes_session";

/// Backend-only Telegram config. Absent config is a hard startup error rather
/// than a per-request surprise: with mandatory auth, a backend that cannot mint
/// a login is a backend that can do nothing.
#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot API token. A backend secret, handled like the Turso write token — it
    /// never reaches the browser.
    pub bot_token: String,
    /// The bot's `@name`, used to build the deep link.
    pub bot_username: String,
    /// Shared secret Telegram echoes back in `X-Telegram-Bot-Api-Secret-Token`.
    /// Without this the webhook is a public endpoint that anyone can POST a
    /// forged `/start` to, claiming any Telegram id they like — i.e. logging in
    /// as anyone. See [`verify_webhook_origin`].
    pub webhook_secret: String,
}

impl TelegramConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            bot_token: req_env("TELEGRAM_BOT_TOKEN")?,
            bot_username: req_env("TELEGRAM_BOT_USERNAME")?,
            webhook_secret: req_env("TELEGRAM_WEBHOOK_SECRET")?,
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

/// A secret we mint and hand out exactly once.
fn mint_secret() -> String {
    let mut buf = [0u8; SECRET_BYTES];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Hash a secret for storage and lookup.
///
/// SHA-256 with no salt or KDF is the right call *here specifically*: these are
/// 256 bits of CSPRNG output, not passwords. There is no dictionary to attack
/// and nothing to guess, so a KDF would add latency to every request while
/// defending against an attack that cannot happen. What this does defend is the
/// case that matters — a leaked database yields no usable credential.
///
/// Hashing also makes the digest the lookup key, so a secret is never compared
/// in application code and there is no timing signal to leak.
fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Serialize)]
pub struct StartResponse {
    /// The deep link to show as a link or QR code.
    pub link: String,
    /// Redeems the session once the attempt is claimed. Never put this in the
    /// link — see the module docs and [`start`].
    pub poll_secret: String,
    pub expires_at: i64,
}

/// `POST /api/auth/start` — mint a login attempt.
///
/// Returns two secrets with different jobs. The **nonce** goes in the link and
/// is shareable by design: the user may screenshot it, and #20 wants the bot
/// posting links into group chats. The **poll secret** never leaves this browser
/// and is what redeems the session.
///
/// Splitting them is load-bearing. If the nonce also redeemed the session,
/// anyone who saw the link — a group chat, a shoulder, a screenshot — could poll
/// with it and walk off with the session it mints. Split, sharing a link only
/// lets someone claim the attempt with their *own* Telegram id.
pub async fn start(State(state): State<AppState>) -> Result<Json<StartResponse>, AppError> {
    let nonce = mint_secret();
    let poll_secret = mint_secret();
    let expires_at = now() + NONCE_TTL_SECS;

    state
        .db
        .execute(
            "INSERT INTO login_attempts (nonce_hash, poll_secret_hash, expires_at)
             VALUES (?1, ?2, ?3)",
            libsql::params![hash_secret(&nonce), hash_secret(&poll_secret), expires_at],
        )
        .await
        .map_err(|e| AppError::Internal(format!("could not mint login: {e}")))?;

    Ok(Json(StartResponse {
        link: format!("https://t.me/{}?start={nonce}", state.telegram.bot_username),
        poll_secret,
        expires_at,
    }))
}

#[derive(Debug, Deserialize)]
pub struct PollRequest {
    pub poll_secret: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollResponse {
    /// Nobody has tapped the link yet.
    Pending,
    /// Claimed. The session rides in a `Set-Cookie`, deliberately **not** in this
    /// body: the client never holds the token, so script cannot read it.
    Ready { username: Option<String> },
    /// The attempt ran out of time. Mint a new one.
    Expired,
}

/// `POST /api/auth/poll` — redeem a claimed attempt for a session.
///
/// Single-use: the attempt is deleted as it is redeemed, so a replayed poll
/// secret gets nothing. The session token is returned **only** as an `HttpOnly`
/// cookie and stored only as a hash, so it exists in exactly two places the
/// client cannot read: the browser's cookie jar, and a digest in our database.
pub async fn poll(
    State(state): State<AppState>,
    Json(req): Json<PollRequest>,
) -> Result<Response, AppError> {
    let secret_hash = hash_secret(&req.poll_secret);

    let mut rows = state
        .db
        .query(
            "SELECT nonce_hash, expires_at, telegram_user_id, username
             FROM login_attempts WHERE poll_secret_hash = ?1",
            libsql::params![secret_hash.clone()],
        )
        .await
        .map_err(|e| AppError::Internal(format!("poll failed: {e}")))?;

    // An unknown secret and an expired one are the same answer on purpose: a
    // caller guessing secrets learns nothing about which ones ever existed.
    let Some(row) = rows
        .next()
        .await
        .map_err(|e| AppError::Internal(format!("poll failed: {e}")))?
    else {
        return Ok(Json(PollResponse::Expired).into_response());
    };

    let nonce_hash: String = row.get(0).map_err(row_err)?;
    let expires_at: i64 = row.get(1).map_err(row_err)?;
    let telegram_user_id: Option<String> = row.get(2).map_err(row_err)?;
    let username: Option<String> = row.get(3).map_err(row_err)?;

    if now() >= expires_at {
        // Expiry is enforced here, not only by a sweep: a sweep that has not run
        // yet must never mean an expired attempt still works.
        delete_attempt(&state.db, &nonce_hash).await?;
        return Ok(Json(PollResponse::Expired).into_response());
    }

    let Some(telegram_user_id) = telegram_user_id else {
        return Ok(Json(PollResponse::Pending).into_response());
    };

    let user_id = upsert_user(&state.db, &telegram_user_id, username.as_deref()).await?;
    let token = issue_session(&state.db, user_id).await?;
    delete_attempt(&state.db, &nonce_hash).await?;

    let mut res = Json(PollResponse::Ready { username }).into_response();
    res.headers_mut().insert(
        SET_COOKIE,
        session_cookie(&token, &state.cookie)
            .parse()
            .map_err(|e| AppError::Internal(format!("bad cookie: {e}")))?,
    );
    Ok(res)
}

/// How the session cookie is scoped. Differs between dev and prod in ways that
/// must not be guessed: `Secure` would make the cookie invisible over plain
/// `http://localhost`, and a `Domain` of the production site would make it
/// invisible in dev.
#[derive(Debug, Clone)]
pub struct CookieConfig {
    /// Parent domain both services live under, e.g. `lehlehleh.com`, so
    /// `recipes.` and `api.recipes.` share one cookie. `None` in dev, which
    /// scopes it to the host that set it.
    ///
    /// This is exactly what `onrender.com` could not do: it is on the Public
    /// Suffix List, so browsers reject a `Domain` of it — two Render subdomains
    /// are different *sites*, not one. A domain we own is what makes a shared
    /// cookie legal at all.
    pub domain: Option<String>,
    /// Off only for local http. Anything reachable must set this.
    pub secure: bool,
}

impl CookieConfig {
    /// `COOKIE_DOMAIN` unset means dev: host-scoped and non-`Secure`.
    pub fn from_env() -> Self {
        let domain = std::env::var("COOKIE_DOMAIN")
            .ok()
            .filter(|d| !d.trim().is_empty());
        Self {
            secure: domain.is_some(),
            domain,
        }
    }
}

/// Build the session cookie.
///
/// `HttpOnly` is the point of the whole transport: script cannot read it, so an
/// XSS cannot exfiltrate the session for offline or later reuse — which is
/// exactly what a token in JS-reachable storage cannot prevent.
///
/// `SameSite=Lax` is sufficient rather than `None` **because we own the domain**:
/// `recipes.lehlehleh.com` and `api.recipes.lehlehleh.com` share a registrable
/// domain, so requests between them are same-site and the cookie rides along —
/// including the #20 WebSocket handshake, which is cross-origin but same-site.
/// `SameSite=None` would mean a third-party cookie, which Safari and Firefox
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

async fn delete_attempt(conn: &Connection, nonce_hash: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM login_attempts WHERE nonce_hash = ?1",
        libsql::params![nonce_hash.to_owned()],
    )
    .await
    .map_err(|e| AppError::Internal(format!("could not clear login attempt: {e}")))?;
    Ok(())
}

/// Find or create the user behind a Telegram id, refreshing their display name.
///
/// Keyed on the numeric id, never the username — a username can be released and
/// claimed by someone else, so a username-keyed account could be inherited.
async fn upsert_user(
    conn: &Connection,
    telegram_user_id: &str,
    username: Option<&str>,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO users (telegram_user_id, username) VALUES (?1, ?2)
         ON CONFLICT(telegram_user_id) DO UPDATE SET
            username = COALESCE(excluded.username, users.username)",
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
    #[allow(dead_code)]
    pub telegram_user_id: String,
}

/// Reject any request without a live session.
///
/// Auth is mandatory (#25), so this wraps everything except `/health` and the
/// login endpoints themselves. Since #29 the client drives ingestion and the
/// server performs it, which means `/api/ingest` **is what a search does** —
/// gating it gates search, deliberately.
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
            "SELECT s.user_id, s.expires_at, u.telegram_user_id
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

    // Checked on read, not left to a sweep: an expired session must be dead the
    // moment it expires, whatever housekeeping has or has not run.
    if now() >= expires_at {
        return Err(AppError::Unauthorized("unknown or expired session".into()));
    }

    req.extensions_mut().insert(CurrentUser {
        id: user_id,
        telegram_user_id,
    });
    Ok(next.run(req).await)
}

/// Pull the session out of the `Cookie` header.
///
/// Hand-parsed rather than pulling in a cookie crate: this reads one name from a
/// header, and the parse is the security boundary, so it is worth being able to
/// see all of it. A `Cookie` header is `a=1; b=2`, and a name must match whole —
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

/// Is this request really from Telegram?
///
/// The webhook URL is public, so without this anyone could POST a handcrafted
/// `/start <nonce>` naming any `telegram_user_id` — forging a login for an
/// account they do not own. Telegram echoes the secret we registered with
/// `setWebhook` in `X-Telegram-Bot-Api-Secret-Token`.
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

/// `POST /api/telegram/webhook` — the bot receiving `/start <nonce>`.
///
/// Always answers 200. Telegram retries non-2xx, so returning an error for a
/// nonce we do not like would earn a retry storm for something that will never
/// succeed. A forged origin is the exception: that gets 401 and no work.
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
    let (Some(text), Some(from)) = (message.text, message.from) else {
        return Ok(StatusCode::OK);
    };

    let Some(nonce) = text.strip_prefix("/start ").map(str::trim) else {
        // A bare /start (or any other message) is someone poking the bot.
        if let Some(chat) = message.chat {
            reply(
                &state,
                chat.id,
                "Open the site and tap the login link there.",
            )
            .await;
        }
        return Ok(StatusCode::OK);
    };

    let claimed = claim_attempt(&state.db, nonce, from.id, from.username.as_deref()).await?;
    if let Some(chat) = message.chat {
        let msg = if claimed {
            "Signed in. Head back to the site."
        } else {
            "That login link is expired or already used. Grab a fresh one from the site."
        };
        reply(&state, chat.id, msg).await;
    }
    Ok(StatusCode::OK)
}

/// Bind a Telegram id to an unclaimed, unexpired attempt.
///
/// The claim is the `UPDATE ... WHERE claimed_at IS NULL` itself, so two taps of
/// the same link race in the database rather than in our code: exactly one wins,
/// and the loser is told it is used.
async fn claim_attempt(
    conn: &Connection,
    nonce: &str,
    telegram_user_id: i64,
    username: Option<&str>,
) -> Result<bool, AppError> {
    let changed = conn
        .execute(
            "UPDATE login_attempts
                SET telegram_user_id = ?1, username = ?2, claimed_at = unixepoch()
              WHERE nonce_hash = ?3
                AND claimed_at IS NULL
                AND expires_at > unixepoch()",
            libsql::params![
                telegram_user_id.to_string(),
                username.map(str::to_owned),
                hash_secret(nonce),
            ],
        )
        .await
        .map_err(|e| AppError::Internal(format!("claim failed: {e}")))?;
    Ok(changed > 0)
}

/// Best-effort reply. A failed courtesy message must not fail the login that
/// already succeeded, so this logs and moves on.
async fn reply(state: &AppState, chat_id: i64, text: &str) {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        state.telegram.bot_token
    );
    // Deliberately not the SSRF-guarded client: that one exists to fetch
    // attacker-influenced URLs, and this is a fixed, first-party API endpoint.
    if let Err(e) = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
        .send()
        .await
    {
        tracing::warn!("could not reply to chat {chat_id}: {e}");
    }
}

/// Mint a real session for a Telegram id, for tests that need to be *past* the
/// gate. Goes through the same code the login path does, so a test cannot
/// accidentally prove the gate against a session the real flow could not issue.
#[cfg(test)]
pub async fn issue_test_session(conn: &Connection, telegram_user_id: &str) -> String {
    let user = upsert_user(conn, telegram_user_id, None).await.unwrap();
    issue_session(conn, user).await.unwrap()
}

/// Delete expired attempts and sessions. Housekeeping only — both are already
/// refused on read, so this reclaims rows rather than enforcing anything.
pub async fn sweep_expired(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM login_attempts WHERE expires_at <= unixepoch()",
        (),
    )
    .await?;
    conn.execute("DELETE FROM sessions WHERE expires_at <= unixepoch()", ())
        .await?;
    Ok(())
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

    async fn insert_attempt(conn: &Connection, nonce: &str, poll_secret: &str, expires_in: i64) {
        conn.execute(
            "INSERT INTO login_attempts (nonce_hash, poll_secret_hash, expires_at)
             VALUES (?1, ?2, ?3)",
            libsql::params![
                hash_secret(nonce),
                hash_secret(poll_secret),
                now() + expires_in
            ],
        )
        .await
        .unwrap();
    }

    /// Secrets must be unguessable and never repeat.
    #[test]
    fn minted_secrets_are_random_and_link_safe() {
        let a = mint_secret();
        let b = mint_secret();
        assert_ne!(a, b);
        // Telegram caps `start` at 64 chars of [A-Za-z0-9_-].
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// The stored form must not be the secret itself — a DB leak must not be a
    /// login.
    #[test]
    fn secrets_are_hashed_at_rest() {
        let secret = mint_secret();
        let hashed = hash_secret(&secret);
        assert_ne!(hashed, secret);
        assert_eq!(hashed, hash_secret(&secret), "hashing must be stable");
        assert_ne!(hashed, hash_secret(&mint_secret()));
    }

    /// The whole point of the webhook secret: without it, anyone can POST a
    /// forged `/start` claiming any Telegram id, which forges a login.
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

        // Found among others, in any position, with the spaces browsers send.
        h.insert(
            COOKIE,
            HeaderValue::from_static("theme=dark; recipes_session=abc; lang=en"),
        );
        assert_eq!(session_from_cookie(&h).as_deref(), Some("abc"));

        // A name must match WHOLE — a prefix or suffix must not satisfy it, or
        // an attacker-set `xrecipes_session` could stand in for the real one.
        for hostile in [
            "xrecipes_session=abc",
            "recipes_session_x=abc",
            "notrecipes_session=abc",
        ] {
            h.insert(COOKIE, HeaderValue::from_str(hostile).unwrap());
            assert_eq!(session_from_cookie(&h), None, "{hostile} must not match");
        }

        // Present but empty is not a session.
        h.insert(COOKIE, HeaderValue::from_static("recipes_session="));
        assert_eq!(session_from_cookie(&h), None);

        // Junk must not panic or match.
        h.insert(COOKIE, HeaderValue::from_static("garbage"));
        assert_eq!(session_from_cookie(&h), None);
    }

    /// The whole reason for choosing a cookie over a bearer token: script must
    /// not be able to read the session.
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
        // which is legal only because we own the domain. `onrender.com` is on the
        // Public Suffix List, so a Domain of it would be rejected outright.
        assert!(c.contains("Domain=lehlehleh.com"), "{c}");
        assert!(c.starts_with("recipes_session=tok123;"), "{c}");
    }

    /// Dev is http on localhost: `Secure` would make the cookie invisible, and a
    /// production `Domain` would too.
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

    /// A cookie set by `poll` must be readable back by the gate — the two parse
    /// the same value from opposite ends, so a mismatch would lock everyone out.
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
        // What a browser would echo back: just the name=value pair.
        let pair = set.split(';').next().unwrap().to_owned();
        let mut h = HeaderMap::new();
        h.insert(COOKIE, HeaderValue::from_str(&pair).unwrap());
        assert_eq!(session_from_cookie(&h).as_deref(), Some(token.as_str()));
    }

    #[tokio::test]
    async fn claim_binds_the_telegram_id() {
        let conn = conn().await;
        insert_attempt(&conn, "nonce-a", "poll-a", 900).await;

        assert!(claim_attempt(&conn, "nonce-a", 4242, Some("dave"))
            .await
            .unwrap());

        let mut rows = conn
            .query(
                "SELECT telegram_user_id, username FROM login_attempts WHERE nonce_hash = ?1",
                libsql::params![hash_secret("nonce-a")],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "4242");
        assert_eq!(row.get::<String>(1).unwrap(), "dave");
    }

    /// Single-use: two people tapping the same shared link cannot both claim it.
    #[tokio::test]
    async fn an_attempt_can_only_be_claimed_once() {
        let conn = conn().await;
        insert_attempt(&conn, "nonce-a", "poll-a", 900).await;

        assert!(claim_attempt(&conn, "nonce-a", 1, None).await.unwrap());
        assert!(
            !claim_attempt(&conn, "nonce-a", 2, None).await.unwrap(),
            "a second claim must lose"
        );

        // And the first claimer keeps it.
        let mut rows = conn
            .query(
                "SELECT telegram_user_id FROM login_attempts WHERE nonce_hash = ?1",
                libsql::params![hash_secret("nonce-a")],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "1");
    }

    #[tokio::test]
    async fn an_expired_attempt_cannot_be_claimed() {
        let conn = conn().await;
        insert_attempt(&conn, "old", "poll-old", -1).await;
        assert!(!claim_attempt(&conn, "old", 1, None).await.unwrap());
    }

    #[tokio::test]
    async fn an_unknown_nonce_claims_nothing() {
        let conn = conn().await;
        insert_attempt(&conn, "real", "poll-real", 900).await;
        assert!(!claim_attempt(&conn, "guess", 1, None).await.unwrap());
    }

    /// Identity is the numeric id: a user who renames stays the same account,
    /// rather than acquiring a second one.
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

    /// Two Telegram ids are two people, even if one later takes the other's old
    /// username.
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

    /// Housekeeping reclaims rows; it is not what makes expiry work.
    #[tokio::test]
    async fn sweep_clears_expired_rows_only() {
        let conn = conn().await;
        insert_attempt(&conn, "fresh", "poll-fresh", 900).await;
        insert_attempt(&conn, "stale", "poll-stale", -1).await;

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
            .query("SELECT count(*) FROM login_attempts", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1
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
