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
//!    along with your Telegram id — that id is the login. Sending `/login` in an
//!    already-open chat does the same: Telegram only auto-sends `/start` on the
//!    first open, so `/login` is the typeable command for signing in again.
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
//! ## What a deep link may carry, and what it may not (#206)
//!
//! The same cost bites on one phone: a QR scan opens the *system* browser, which
//! holds no session however signed in that phone's Telegram is, so an invite always
//! meets the login screen — and a login that landed at home lost the invite. So the
//! **destination** travels with the login. `?start=<payload>` arrives here as the
//! message `/start <payload>`, and [`start_payload`] hands it to [`completion_link`]
//! to sit *beside* the secret in the bot's reply.
//!
//! It carries **where**, never **who**. Anyone at all can `/start` this bot with any
//! payload, so it is deliberately inert: not minted, not hashed, not stored, not read
//! at redemption. Nothing above this line changes shape when it is present. All it
//! can do is move a browser that has already signed *itself* in — and even that only
//! once the browser has checked it is a same-origin relative path, which it does at
//! the point it navigates (`frontend/src/lib/destination.ts`), never on the strength
//! of having come through us.
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

/// The key that guards `/api/ingest`, or `None` where this deployment has none.
///
/// Ingest is a server-driven corpus sync (#49) triggered by a schedule, not a
/// person — so it authenticates a **machine**, not a session. Like the webhook
/// secret this is a shared secret we mint and hold in the environment; unlike it,
/// the convention is ours, so we use the standard `Authorization: Bearer` rather
/// than a bespoke `X-` header (the `X-` prefix is deprecated for new headers, and
/// proxies and log pipelines redact `Authorization` by convention).
///
/// **Optional, unlike the secrets above — this one must not be fatal.** Those are
/// startup errors because without them nobody can log in at all, so the process
/// has nothing to serve. This one guards a single background endpoint: lose it
/// and a scheduled sync stops, which leaves the corpus stale, not unreadable.
/// Exiting over it would escalate one paused feature into a total outage — login,
/// reads and `/health` gone with it — so the app boots without it, says so
/// loudly, and keeps serving.
///
/// **Absent means closed, never open.** This is an `Option` rather than a
/// `String` that defaults to empty precisely so the unconfigured case cannot be
/// compared against: an empty expected key makes `Authorization: Bearer ` — the
/// scheme with nothing after it — a *matching* credential, so a missing config
/// would silently open the endpoint instead of closing it. [`require_api_key`]
/// rejects on `None` before any comparison exists to get wrong. For the same
/// reason set-but-empty reads as `None`: `INGEST_API_KEY=""` is a missing key,
/// not a secret that happens to be the empty string.
pub fn ingest_key_from_env() -> Option<String> {
    std::env::var("INGEST_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// The configured admin's Telegram id, or `None` when unset.
///
/// Identity is the numeric Telegram id, **not** the username — the same rule the
/// login path follows ([`upsert_user`]): a username can be released and reclaimed by
/// someone else, so keying admin on it could hand admin to a different person. Find
/// your id in `GET /api/me`.
pub fn admin_id_from_env() -> Option<String> {
    std::env::var("ADMIN_TELEGRAM_USER_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// Is this Telegram id the configured admin? **False when no admin is configured** —
/// an unset `ADMIN_TELEGRAM_USER_ID` grants admin to nobody (fail closed), the same
/// stance as the ingest key. A plain compare is fine: the id is not a secret (the
/// session already authenticated the user), so there is no timing signal worth
/// hiding.
pub fn is_admin(state: &AppState, telegram_user_id: &str) -> bool {
    state
        .admin_id
        .as_deref()
        .is_some_and(|admin| admin == telegram_user_id)
}

/// The chat the bot can reach the admin in, or `None` where this deployment has
/// nobody to tell (#183).
///
/// **The same id, not a second variable.** In a Telegram one-to-one chat the chat id
/// *is* the user's id, so the admin already configured for the health dashboard is
/// already an address. A separate `ADMIN_TELEGRAM_CHAT_ID` would be a second identity
/// for one person, free to drift out of step with the first, and a drifted alarm
/// address is a silent one.
///
/// A bot cannot open a conversation (`Forbidden: bot can't initiate conversation with
/// a user`), so an arbitrary id would be unreachable — but this one is not arbitrary:
/// the way anyone learns their Telegram id here is `GET /api/me`, which requires a
/// session, which is minted by the bot in that very chat. Being the admin means having
/// messaged the bot.
///
/// [`is_admin`] compares ids as strings, so an id that is not a number gates the
/// dashboard perfectly well and cannot be messaged. That is a config typo, and the
/// alarm says so loudly rather than degrading into silence.
pub fn admin_chat_id(state: &AppState) -> Option<i64> {
    resolve_admin_chat(state.admin_id.as_deref())
}

/// The rule [`admin_chat_id`] applies, apart from the state that carries it — the
/// [`CookieConfig::parse`] split, and for the same reason: a rule about what a
/// configured value means should be checkable without building a process around it.
///
/// **Positive only.** Telegram user ids are positive; a negative chat id is a group or
/// a channel. An alarm addressed to a group is a broadcast, which this feature
/// explicitly is not — an operational message about the corpus goes to the operator,
/// not to everyone who happens to be in a room with the bot.
fn resolve_admin_chat(configured: Option<&str>) -> Option<i64> {
    let configured = configured?;
    match configured.parse::<i64>() {
        Ok(chat_id) if chat_id > 0 => Some(chat_id),
        _ => {
            tracing::error!(
                "ADMIN_TELEGRAM_USER_ID is `{configured}`, which is not a personal Telegram \
                 id — it is the positive numeric id from GET /api/me, never a username and \
                 never a group. Nothing can be sent to the admin until it is fixed."
            );
            None
        }
    }
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
pub(crate) fn hash_secret(secret: &str) -> String {
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
    let hash = hash.as_str();

    let found = state
        .with_db(move |db| async move {
            let mut rows = db
                .query(
                    "SELECT telegram_user_id, username, expires_at
                     FROM login_completions WHERE completion_hash = ?1",
                    libsql::params![hash.to_owned()],
                )
                .await?;
            match rows.next().await? {
                Some(row) => Ok(Some((
                    row.get::<String>(0)?,
                    row.get::<Option<String>>(1)?,
                    row.get::<i64>(2)?,
                ))),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| AppError::Internal(format!("login lookup failed: {e:#}")))?;

    // An unknown secret and an expired one get the same answer on purpose: a
    // caller guessing secrets learns nothing about which ones ever existed.
    let Some((telegram_user_id, username, expires_at)) = found else {
        return Err(AppError::Unauthorized(
            "that link is expired or used".into(),
        ));
    };

    // Burn it first: redeemed or expired, it must not survive this call.
    state
        .with_db(move |db| async move { delete_completion(&db, hash).await })
        .await
        .map_err(|e| AppError::Internal(format!("could not clear login: {e:#}")))?;

    // Checked here, not left to a sweep: a sweep that has not run yet must never
    // mean an expired credential still works.
    if now() >= expires_at {
        return Err(AppError::Unauthorized(
            "that link is expired or used".into(),
        ));
    }

    let user_id = {
        let telegram_user_id = telegram_user_id.as_str();
        let username = &username;
        state.with_db(move |db| async move {
            upsert_user(&db, telegram_user_id, username.as_deref()).await
        })
    }
    .await
    .map_err(|e| AppError::Internal(format!("could not upsert user: {e:#}")))?;
    // The session token is minted inside the retried closure, so each attempt
    // writes its own row. If an attempt applied but its response was lost, the
    // orphaned row is a hashed token nobody ever saw — unredeemable, and swept
    // when it expires (#130).
    let token = state
        .with_db(move |db| async move { issue_session(&db, user_id).await })
        .await
        .map_err(|e| AppError::Internal(format!("could not issue session: {e:#}")))?;

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

async fn delete_completion(conn: &Connection, hash: &str) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM login_completions WHERE completion_hash = ?1",
        libsql::params![hash.to_owned()],
    )
    .await?;
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
) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO users (telegram_user_id, username) VALUES (?1, ?2)
         ON CONFLICT(telegram_user_id) DO UPDATE SET username = excluded.username",
        libsql::params![telegram_user_id.to_owned(), username.map(str::to_owned)],
    )
    .await?;

    let mut rows = conn
        .query(
            "SELECT id FROM users WHERE telegram_user_id = ?1",
            libsql::params![telegram_user_id.to_owned()],
        )
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("user vanished after upsert"))?;
    Ok(row.get::<i64>(0)?)
}

async fn issue_session(conn: &Connection, user_id: i64) -> anyhow::Result<String> {
    let token = mint_secret();
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
        libsql::params![hash_secret(&token), user_id, now() + SESSION_TTL_SECS],
    )
    .await?;
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

    // Scoped deliberately: the whole read happens inside the `with_db` closure. A
    // result set holds an open statement, and an open statement holds a read on the
    // database — so a `rows` living to the end of this function would hold one for
    // the whole request, across `next.run` below. Every connection used to be the
    // same connection, which SQLite forgives; they are now separate, and a handler
    // trying to write behind a reader that never finishes is a deadlock, not a slow
    // query. Read what is needed, then let go. This was also the incident's loudest
    // failure ("session lookup failed" on every request, #130) — as a pure read it
    // retries freely.
    let token = token.as_str();
    let looked_up = state
        .with_db(move |db| async move {
            let mut rows = db
                .query(
                    "SELECT s.user_id, s.expires_at, u.telegram_user_id, u.username
                     FROM sessions s JOIN users u ON u.id = s.user_id
                     WHERE s.token_hash = ?1",
                    libsql::params![hash_secret(token)],
                )
                .await?;
            match rows.next().await? {
                Some(row) => Ok(Some((
                    row.get::<i64>(0)?,
                    row.get::<i64>(1)?,
                    row.get::<String>(2)?,
                    row.get::<Option<String>>(3)?,
                ))),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| AppError::Internal(format!("session lookup failed: {e:#}")))?;

    let Some((user_id, expires_at, telegram_user_id, username)) = looked_up else {
        return Err(AppError::Unauthorized("unknown or expired session".into()));
    };

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
pub async fn me(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Json<MeResponse> {
    let is_admin = is_admin(&state, &user.telegram_user_id);
    Json(MeResponse {
        telegram_user_id: user.telegram_user_id,
        username: user.username,
        is_admin,
    })
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub telegram_user_id: String,
    pub username: Option<String>,
    /// Whether this user is the configured admin — the frontend uses it to show the
    /// admin-only health dashboard. Not a security boundary by itself: the admin
    /// endpoints re-check server-side; this only decides what the UI offers.
    pub is_admin: bool,
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
        let token = token.as_str();
        state
            .with_db(move |db| async move {
                // A delete is idempotent, so a retried logout is still one logout.
                db.execute(
                    "DELETE FROM sessions WHERE token_hash = ?1",
                    libsql::params![hash_secret(token)],
                )
                .await?;
                Ok(())
            })
            .await
            .map_err(|e| AppError::Internal(format!("logout failed: {e:#}")))?;
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
    /// `private` | `group` | `supergroup` | `channel`. Read because what may be said
    /// depends on who can hear it — an invite belongs in a one-to-one chat and nowhere
    /// else.
    #[serde(rename = "type")]
    pub kind: Option<String>,
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
    // No key here means ingest is *off*, not open: refuse before a comparison
    // exists. Falling through to `verify_bearer` with an empty expected key would
    // make `Authorization: Bearer ` match it — an unset variable would unlock the
    // endpoint rather than lock it. See [`ingest_key_from_env`].
    let Some(expected) = state.ingest_key.as_deref() else {
        tracing::warn!("ingest was called but INGEST_API_KEY is not set — refusing");
        return Err(AppError::Unavailable(
            "ingest is not configured on this server".into(),
        ));
    };
    if !verify_bearer(req.headers(), expected) {
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

/// Does this message ask to sign in? Telegram auto-sends `/start` when the bot is
/// first opened (or its Start button tapped), so that stays the one-tap path;
/// `/login` is the typeable alias for signing in again from an existing chat, where
/// there is no Start button to press. Both mint a fresh completion link. A trailing
/// deep-link payload (`/start abc`) or a group-style suffix (`/login@thebot`) still
/// counts.
fn is_login_command(text: &str) -> bool {
    let cmd = text.trim_start();
    cmd.starts_with("/start") || cmd.starts_with("/login")
}

/// Telegram's own ceiling on a `?start=` deep-link payload: 64 characters of
/// `A-Z a-z 0-9 _ -`. Nothing longer or otherwise spelled can have come from a
/// Telegram deep link, so nothing longer or otherwise spelled is carried.
const START_PAYLOAD_MAX: usize = 64;

/// Could Telegram have carried this as a `?start=` payload?
///
/// The charset and the length are Telegram's, and applying them here is this
/// function's whole job: [`completion_link`] splices what comes back into a URL, so
/// the alphabet is also exactly what makes that splice safe — no `&`, no `#`, no
/// `?`, no space, nothing that could add a second parameter to a link the bot is
/// about to send someone.
///
/// It says nothing about *where* the payload points. The backend is a courier here
/// and deliberately not a judge: the destination is a path only the browser can
/// resolve, and it is checked as a same-origin relative path at the point it is
/// used to navigate (`$lib/destination`), not because a hop it took looked right.
fn is_carryable_payload(payload: &str) -> bool {
    !payload.is_empty()
        && payload.len() <= START_PAYLOAD_MAX
        && payload
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The deep-link payload on a login command, when it has one worth carrying (#206).
///
/// `https://t.me/<bot>?start=<payload>` arrives as the message text `/start <payload>`
/// — one command, one argument, and Telegram never sends more — so anything else is
/// not a deep link and gets a bare login, which is exactly today's behaviour. That
/// includes a plain `/start`, the `/start@thebot` a group adds, and `/login`, which is
/// ours rather than Telegram's and so normally arrives typed and bare.
///
/// The command itself is matched the way [`is_login_command`] matches it, on the first
/// whitespace-separated token, so the two cannot disagree about what a login is.
fn start_payload(text: &str) -> Option<&str> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?;
    if !is_login_command(command) {
        return None;
    }
    let payload = parts.next()?;
    // A second argument is not something a deep link can produce, so it is not a deep
    // link. Falling back to a bare login is the safe reading of a message we do not
    // recognise; guessing which token was meant is not.
    if parts.next().is_some() {
        return None;
    }
    is_carryable_payload(payload).then_some(payload)
}

/// The link the bot replies with: the secret that redeems a session and, **beside it,
/// never inside it**, where to land once the cookie is set (#206).
///
/// The `c=` half is byte-for-byte what it has always been, whatever the destination
/// is or is not — which is the property that matters. A destination cannot influence
/// who a session belongs to, because it is not part of the secret, not part of what
/// is hashed, and not part of what `/auth/complete` looks up. It only reaches the
/// browser's own navigation, after that browser has signed *itself* in (#25).
fn completion_link(base: &str, secret: &str, destination: Option<&str>) -> String {
    match destination {
        Some(r) => format!("{base}/auth/finish?c={secret}&r={r}"),
        None => format!("{base}/auth/finish?c={secret}"),
    }
}

/// Does this message ask for an invite to the sender's kitchen? Same tolerance as
/// [`is_login_command`] for Telegram's padding and `@bot` suffixes.
fn is_kitchen_invite_command(text: &str) -> bool {
    text.trim_start().starts_with("/kitcheninvite")
}

/// Is this a one-to-one chat with the bot?
///
/// Absent means private: Telegram always sends the field, and a message with no chat
/// type is not a group we can identify. Being wrong in the cautious direction here
/// would only ever refuse someone their own invite in their own chat.
fn is_private_chat(chat: &Chat) -> bool {
    chat.kind.as_deref().unwrap_or("private") == "private"
}

/// `POST /api/telegram/webhook` — someone pressed Start, or sent `/login`.
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

    if is_kitchen_invite_command(&text) {
        kitchen_invite_reply(&state, &chat, &from).await?;
        return Ok(StatusCode::OK);
    }

    if !is_login_command(&text) {
        send(
            &state,
            chat.id,
            "Send /login to sign in, or /kitcheninvite for a link to your kitchen.",
        )
        .await;
        return Ok(StatusCode::OK);
    }

    let link = login_reply(&state, &from, &text).await?;
    send(
        &state,
        chat.id,
        &format!(
            "Tap to finish signing in. This link is yours alone and lasts 15 minutes:\n{link}"
        ),
    )
    .await;
    Ok(StatusCode::OK)
}

/// Answer `/kitcheninvite` with a fresh link to the sender's own kitchen.
///
/// Refused outside a one-to-one chat, and that refusal is the point. An invite is a
/// live key to a kitchen — everyone in one is an owner of it — so answering the same
/// command in a group would hand that key to every member of the group at once,
/// including whoever forwards the message on. The bot will say so rather than
/// silently do it.
///
/// Identity comes from Telegram alongside a verified webhook secret, so the link is
/// minted for whoever actually sent the message and delivered only to their chat.
async fn kitchen_invite_reply(
    state: &AppState,
    chat: &Chat,
    from: &TelegramUser,
) -> Result<(), AppError> {
    if !is_private_chat(chat) {
        send(
            state,
            chat.id,
            "Not here — an invite is a live key to your kitchen, and anyone reading this group would have it. Message me directly for one.",
        )
        .await;
        return Ok(());
    }

    let user_id = from.id.to_string();
    let user_id = user_id.as_str();
    let minted = state
        .with_db(move |db| async move {
            crate::kitchens::primary_invite(&db, user_id, from.username.as_deref()).await
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let Some((kitchen, token, _expires_at)) = minted else {
        send(
            state,
            chat.id,
            "You have no kitchen to invite anyone to yet.",
        )
        .await;
        return Ok(());
    };

    let base = state.telegram.frontend_base_url.trim_end_matches('/');
    send(
        state,
        chat.id,
        &format!(
            "Anyone who opens this joins {kitchen}, the same as everyone else in it. It lasts 2 hours:\n{base}/kitchens?join={token}"
        ),
    )
    .await;
    Ok(())
}

/// The whole answer to a login command: the link [`webhook`] sends back.
///
/// A seam rather than an inline expression, because this composition **is** the
/// feature (#206) — a deep link's payload becomes the destination on the link, and a
/// bare `/start` produces exactly the link it always did. Written out here it is a
/// thing a test can hold; folded into [`webhook`] it could only be reached through
/// Telegram's HTTP API, which is the one thing the test could not have.
pub(crate) async fn login_reply(
    state: &AppState,
    from: &TelegramUser,
    text: &str,
) -> Result<String, AppError> {
    // A deep link may say where the person was heading — a plan they scanned an
    // invite to, most of the time. It rides along; it is not part of the login.
    mint_completion(state, from, start_payload(text)).await
}

/// Mint a completion secret for a **known** Telegram user and build their link.
///
/// The user is not something a caller chose: it arrived from Telegram alongside a
/// verified webhook secret.
///
/// `destination` is [`start_payload`]'s answer and is **not part of the login**. It
/// is not minted, not hashed, not stored, and not read back at redemption — nothing
/// below this line changes shape when it is `Some`. It is handed to
/// [`completion_link`] and nowhere else, which is what keeps a payload anybody can
/// send from being able to say anything about whose session comes out (#25).
async fn mint_completion(
    state: &AppState,
    user: &TelegramUser,
    destination: Option<&str>,
) -> Result<String, AppError> {
    // Minted outside the retryable closure: every attempt writes the same hash, so
    // a re-run after a lost response collides on the primary key and fails honestly
    // instead of leaving a second live login link (#130).
    let secret = mint_secret();
    let hash = hash_secret(&secret);
    let hash = hash.as_str();
    let expires_at = now() + COMPLETION_TTL_SECS;
    state
        .with_db(move |db| async move {
            db.execute(
                "INSERT INTO login_completions (completion_hash, telegram_user_id, username, expires_at)
                 VALUES (?1, ?2, ?3, ?4)",
                libsql::params![
                    hash.to_owned(),
                    user.id.to_string(),
                    user.username.clone(),
                    expires_at
                ],
            )
            .await?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(format!("could not mint login: {e:#}")))?;

    // Cheap, and it is why this table cannot grow without bound: a login is the
    // only thing that writes it, and each one clears the dead rows behind it. No
    // scheduler, and no anonymous caller who could outrun it — reaching here at
    // all costs an authenticated Telegram round trip.
    if let Err(e) = state
        .with_db(move |db| async move { sweep_expired(&db).await })
        .await
    {
        tracing::warn!("could not sweep expired auth rows: {e:#}");
    }

    Ok(completion_link(
        &state.telegram.frontend_base_url,
        &secret,
        destination,
    ))
}

/// Send a message as the bot to one chat, and say whether it left. Best-effort: it
/// logs and never propagates, because a failed courtesy message must not fail a login
/// already minted, and a failed alarm must not 500 the check that raised it.
///
/// **The answer is load-bearing for the alarm (#183) and ignored by the login path.**
/// A run is marked reported only once its message is actually out, so that a lost
/// message costs a repeat rather than a silence — repeating is annoying, and silence
/// is the bug the alarm exists to end.
///
/// **A refusal is not a delivery.** Telegram answers `Forbidden: bot can't initiate
/// conversation with a user`, a revoked token, or an unknown chat with an HTTP error
/// carrying a JSON body — which `reqwest` hands back as `Ok`, because the *request*
/// succeeded. Reading only the transport, as this used to, would mark a run told when
/// nobody was told and lose the alarm for good. Status is what says delivered.
pub(crate) async fn send(state: &AppState, chat_id: i64, text: &str) -> bool {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        state.telegram.bot_token
    );
    // Deliberately not the SSRF-guarded client: that exists to fetch
    // attacker-influenced URLs, and this is a fixed first-party endpoint.
    let sent = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true
        }))
        .send()
        .await;
    match sent {
        Ok(response) if delivered(response.status()) => true,
        Ok(response) => {
            let status = response.status();
            // The body carries Telegram's own `description`, which is the only thing
            // that distinguishes "wrong token" from "that chat has never opened".
            let body = response.text().await.unwrap_or_default();
            tracing::warn!("telegram refused a message to chat {chat_id}: {status} {body}");
            false
        }
        Err(e) => {
            tracing::warn!("could not reach telegram for chat {chat_id}: {e}");
            false
        }
    }
}

/// Did Telegram take the message? Split out of [`send`] so the rule is checkable
/// without a network, because it is the rule a network test would be written for: a
/// refusal arrives as a perfectly successful HTTP round trip, and reading only the
/// transport is what would let an unsent alarm be marked told.
fn delivered(status: StatusCode) -> bool {
    status.is_success()
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

    /// `/login` is an alias of `/start` (both sign in); a deep-link payload or an
    /// `@mention` suffix still counts; anything else does not.
    #[test]
    fn login_command_accepts_start_and_login() {
        assert!(is_login_command("/start"));
        assert!(is_login_command("/login"));
        assert!(is_login_command("  /login"), "Telegram may pad");
        assert!(is_login_command("/start deadbeef"), "deep-link payload");
        assert!(
            is_login_command("/login@lehlehlehbot"),
            "group @mention suffix"
        );
        assert!(!is_login_command("/help"));
        assert!(!is_login_command("hello"));
        assert!(!is_login_command("start"), "a slash is required");
    }

    /// The payload `frontend/src/lib/destination.ts` produces for a pick invite, and
    /// the path it decodes back to. Written out rather than computed so the two halves
    /// of the round trip meet on a literal: this side proves the payload reaches the
    /// bot's link unchanged, and `destination.test.ts` proves it decodes to this path.
    const INVITE_PAYLOAD: &str = "L3BpY2svOWY0YjJjMWQ4ZTdhNjA1Mw";

    /// A `/start` with nothing after it is the whole of today's behaviour and stays
    /// exactly it — including the `@thebot` suffix a group adds and our own typed
    /// `/login`, neither of which is a deep link.
    #[test]
    fn a_bare_login_command_carries_no_destination() {
        for bare in [
            "/start",
            "/login",
            "  /start",
            "/start ",
            "/start@lehlehlehbot",
            "/login@lehlehlehbot",
        ] {
            assert_eq!(start_payload(bare), None, "{bare:?} carries nothing");
        }
        assert_eq!(
            start_payload("/help L3BpY2sveA"),
            None,
            "a payload on something that is not a login is not a login"
        );
    }

    /// `https://t.me/<bot>?start=<payload>` arrives as `/start <payload>`.
    #[test]
    fn a_deep_link_payload_is_carried() {
        assert_eq!(
            start_payload(&format!("/start {INVITE_PAYLOAD}")),
            Some(INVITE_PAYLOAD)
        );
        assert_eq!(
            start_payload(&format!("/start@lehlehlehbot {INVITE_PAYLOAD}")),
            Some(INVITE_PAYLOAD),
            "a group adds the @mention and the deep link still works"
        );
        assert_eq!(
            start_payload(&format!("  /start   {INVITE_PAYLOAD}  ")),
            Some(INVITE_PAYLOAD),
            "Telegram pads"
        );
        assert_eq!(
            start_payload(&format!("/login {INVITE_PAYLOAD}")),
            Some(INVITE_PAYLOAD),
            "/login is our alias for the same thing, so it carries the same thing"
        );
    }

    /// The rule itself, apart from the message it was pulled out of.
    ///
    /// Telegram's alphabet is also what makes [`completion_link`]'s splice safe, so
    /// this is two properties in one predicate: nothing here could have arrived from
    /// a deep link, and nothing here could add a second parameter to a URL.
    #[test]
    fn a_carryable_payload_is_telegrams_own_alphabet_and_length() {
        assert!(is_carryable_payload(INVITE_PAYLOAD));
        assert!(is_carryable_payload("A"), "one character is a payload");
        assert!(is_carryable_payload("a_b"), "underscore is base64url");
        assert!(is_carryable_payload("a-b"), "so is hyphen");
        assert!(is_carryable_payload("09azAZ"));

        assert!(
            !is_carryable_payload(""),
            "nothing at all is not a destination"
        );
        for out in ["a+b", "a/b", "a=", "a b", "a&b", "a?b", "a#b", "a.b", "é"] {
            assert!(!is_carryable_payload(out), "{out:?} is not base64url");
        }

        // Written out rather than as `START_PAYLOAD_MAX`: the ceiling is Telegram's
        // documented one, so here the constant is what is under test and cannot also
        // be the oracle for itself.
        assert_eq!(START_PAYLOAD_MAX, 64);
        assert!(is_carryable_payload(&"A".repeat(64)));
        assert!(
            !is_carryable_payload(&"A".repeat(65)),
            "64 is the ceiling Telegram states, so 65 never came from Telegram"
        );
    }

    /// The alphabet is Telegram's, and it is also what makes the splice into the
    /// completion URL safe: none of these could add a second query parameter to a
    /// link the bot is about to send someone.
    #[test]
    fn a_payload_telegram_could_not_have_sent_is_dropped() {
        for impossible in [
            "a&r=b", "a?b", "a#b", "a/b", "a+b", "a.b", "a=", "a%2Fb", "a\"b",
        ] {
            assert_eq!(
                start_payload(&format!("/start {impossible}")),
                None,
                "{impossible:?} is not a Telegram start payload"
            );
        }

        let longest = "A".repeat(START_PAYLOAD_MAX);
        assert_eq!(
            start_payload(&format!("/start {longest}")).map(str::len),
            Some(START_PAYLOAD_MAX),
            "64 characters is the ceiling, not past it"
        );
        let too_long = "A".repeat(START_PAYLOAD_MAX + 1);
        assert_eq!(start_payload(&format!("/start {too_long}")), None);

        assert_eq!(
            start_payload("/start one two"),
            None,
            "a deep link sends one argument, so two is not a deep link"
        );
    }

    /// **The redemption half of the link does not move.** This is the property that
    /// keeps #25 intact through #206: a destination anybody can send the bot rides
    /// beside the secret and never inside it, so the string `/auth/complete` looks up
    /// — and the digest `login_completions` was keyed on — is the same either way.
    #[test]
    fn a_destination_leaves_the_secret_byte_identical() {
        let base = "https://recipes.lehlehleh.com";
        let secret = mint_secret();

        let bare = completion_link(base, &secret, None);
        let carried = completion_link(base, &secret, Some(INVITE_PAYLOAD));

        assert_eq!(bare, format!("{base}/auth/finish?c={secret}"));
        assert_eq!(carried, format!("{bare}&r={INVITE_PAYLOAD}"));
        assert!(
            carried.starts_with(&bare),
            "the destination is appended after the secret, never woven into it"
        );

        let c_of = |link: &str| {
            link.split_once("?c=")
                .map(|(_, rest)| rest.split('&').next().unwrap().to_owned())
        };
        assert_eq!(c_of(&bare).as_deref(), Some(secret.as_str()));
        assert_eq!(
            c_of(&bare),
            c_of(&carried),
            "the same secret redeems the same session with or without a destination"
        );
        assert_eq!(
            hash_secret(&c_of(&bare).unwrap()),
            hash_secret(&c_of(&carried).unwrap()),
            "so the row the redemption looks up is the same row"
        );
    }

    #[test]
    fn the_kitchen_invite_command_is_recognised() {
        assert!(is_kitchen_invite_command("/kitcheninvite"));
        assert!(
            is_kitchen_invite_command("  /kitcheninvite"),
            "Telegram pads"
        );
        assert!(
            is_kitchen_invite_command("/kitcheninvite@lehlehlehbot"),
            "group-style @mention suffix"
        );
        assert!(!is_kitchen_invite_command("/kitchen"));
        assert!(
            !is_login_command("/kitcheninvite"),
            "asking for an invite is not asking to sign in"
        );
    }

    /// An invite is a live key to a kitchen, and everyone in a kitchen is an owner of
    /// it — so where the bot is willing to say one out loud is a security boundary, not
    /// a nicety. Anything that is not a one-to-one chat is a room with an audience.
    #[test]
    fn an_invite_is_only_for_a_one_to_one_chat() {
        let private = Chat {
            id: 1,
            kind: Some("private".into()),
        };
        assert!(is_private_chat(&private));

        for shared in ["group", "supergroup", "channel"] {
            let chat = Chat {
                id: 1,
                kind: Some(shared.into()),
            };
            assert!(
                !is_private_chat(&chat),
                "{shared} has an audience, so no invite goes into it"
            );
        }

        // Telegram always sends the field; a message without one is not a group we can
        // identify, and treating it as private only ever costs someone their own link.
        let unknown = Chat { id: 1, kind: None };
        assert!(is_private_chat(&unknown));
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

    /// Who the run alarm (#183) can be sent to, from what is configured. Nobody is a
    /// perfectly good answer — the alarm degrades to a log — but *the wrong somebody*
    /// is not, so everything that is not one person's own chat resolves to `None`.
    #[test]
    fn an_admin_chat_is_one_persons_own_chat_or_nobody() {
        assert_eq!(resolve_admin_chat(Some("4242")), Some(4242));
        assert_eq!(
            resolve_admin_chat(None),
            None,
            "no admin configured is nobody, never a default and never everybody"
        );
        assert_eq!(
            resolve_admin_chat(Some("thedavidmeister")),
            None,
            "a username is not an id — it is also reassignable, which is why it is not identity here"
        );
        assert_eq!(
            resolve_admin_chat(Some("-1001234567890")),
            None,
            "a negative id is a group or channel, and this alarm is never a broadcast"
        );
        assert_eq!(resolve_admin_chat(Some("0")), None, "nor is zero a person");
        assert_eq!(
            resolve_admin_chat(Some("42 42")),
            None,
            "and a half-parsed id is not half an address"
        );
    }

    /// **A refusal arrives as a successful HTTP round trip.** Telegram answers a bot
    /// that may not write to a chat, or a revoked token, with an error status and a
    /// JSON body — the request itself worked. Only the status says delivered, and the
    /// run alarm marks a run reported on this answer, so reading it wrongly is how an
    /// alarm gets marked told with nobody told.
    #[test]
    fn only_a_success_status_counts_as_delivered() {
        assert!(delivered(StatusCode::OK));
        assert!(
            !delivered(StatusCode::FORBIDDEN),
            "`bot can't initiate conversation with a user` is not a delivery"
        );
        assert!(
            !delivered(StatusCode::UNAUTHORIZED),
            "a revoked bot token is not a delivery"
        );
        assert!(
            !delivered(StatusCode::BAD_REQUEST),
            "`chat not found` is not a delivery"
        );
        assert!(
            !delivered(StatusCode::TOO_MANY_REQUESTS),
            "a rate limit is not a delivery"
        );
        assert!(
            !delivered(StatusCode::INTERNAL_SERVER_ERROR),
            "telegram having a bad minute is not a delivery"
        );
    }
}
