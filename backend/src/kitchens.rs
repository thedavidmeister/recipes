//! Kitchens (#72): the shared space — an owner and invited guests — that scopes the
//! meal flow, with an equipment list and a pantry.
//!
//! A kitchen is a durable group (unlike a pick's ephemeral channel, #20): it holds an
//! owner and any guests, the equipment it has, and the stock on hand. Identity is the
//! Telegram id everywhere (#25), so membership is keyed on `telegram_user_id`. Every
//! endpoint here is person-facing and session-gated (the `guarded` router), and reads
//! the caller from the [`CurrentUser`] the session middleware injects.
//!
//! Handlers stay thin; the persistence is pure `anyhow` functions unit-tested against
//! an in-memory DB, mirroring [`crate::session`]. This is the foundation slice —
//! create, invite, membership, and the equipment/pantry inventory. Scoping the meal
//! flow *into* a kitchen (a pick for this kitchen, #20) is a follow-up.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use libsql::Connection;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

/// Mint an opaque id / invite token: `bytes` of CSPRNG output, hex-encoded. The same
/// recipe as a pick channel ([`crate::session`]) and the auth secrets — so collision
/// and guessing are both negligible.
fn mint(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

// --- request/response shapes ---------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct JoinBody {
    token: String,
}

#[derive(Debug, Deserialize)]
pub struct ItemBody {
    item: String,
}

#[derive(Debug, Deserialize)]
pub struct ItemQuery {
    item: String,
}

/// A kitchen in the caller's list — enough to show and select it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KitchenSummary {
    pub id: String,
    pub name: String,
    /// Whether this is the caller's primary — the one assumed unless they switch.
    pub is_primary: bool,
}

/// A member of a kitchen. `username` is a display convenience (may be absent — a
/// Telegram account need not have one); identity is the id. There is no role: everyone
/// in a kitchen is an owner of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    pub telegram_user_id: String,
    pub username: Option<String>,
}

/// A kitchen in full — who is in it and what it holds. No invite: one is minted on
/// request and lives for two hours, so there is nothing standing to hand out here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KitchenDetail {
    pub id: String,
    pub name: String,
    /// Whether this is the **caller's** primary.
    pub is_primary: bool,
    pub members: Vec<Member>,
    pub equipment: Vec<String>,
    pub pantry: Vec<String>,
}

// --- handlers ------------------------------------------------------------------

/// `POST /api/kitchens` — create a kitchen owned by the caller.
pub async fn create(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Json(body): Json<CreateBody>,
) -> Result<Json<KitchenDetail>, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("kitchen name is required".into()));
    }
    let id = create_kitchen(&state.db()?, name, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    load_detail(&state.db()?, &id, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
        .map(Json)
}

/// `GET /api/kitchens` — the kitchens the caller belongs to, primary first.
///
/// Seeing your kitchens is also how you come to have one: nobody can have zero, so if
/// the caller has no primary yet this mints it before answering. Doing it here rather
/// than at signup means the guarantee also reaches people who logged in before there
/// was such a thing as a primary.
pub async fn list(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
) -> Result<Json<Vec<KitchenSummary>>, AppError> {
    ensure_primary(
        &state.db()?,
        &user.telegram_user_id,
        user.username.as_deref(),
    )
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    list_kitchens(&state.db()?, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
        .map(Json)
}

/// `GET /api/kitchens/{id}` — a kitchen in full. A non-member is refused.
pub async fn get(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<KitchenDetail>, AppError> {
    require_member(&state.db()?, &id, &user.telegram_user_id).await?;
    load_detail(&state.db()?, &id, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
        .map(Json)
}

/// `POST /api/kitchens/{id}/name` — rename a kitchen. Any member may.
///
/// Renaming is the only thing you do to a primary kitchen that you would otherwise
/// have done by creating one: it arrives named after you, and this is how it stops
/// being.
pub async fn rename(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<CreateBody>,
) -> Result<Json<KitchenDetail>, AppError> {
    require_member(&state.db()?, &id, &user.telegram_user_id).await?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("kitchen name is required".into()));
    }
    rename_kitchen(&state.db()?, &id, name)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    load_detail(&state.db()?, &id, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
        .map(Json)
}

/// How long an invite is good for. Long enough to hand someone a phone or send a
/// message; short enough that a link which escapes is a problem that ends.
const INVITE_TTL_SECS: i64 = 2 * 60 * 60;

/// How far another process's clock may disagree with ours before we treat what it
/// wrote as impossible rather than merely early. Generous enough that ordinary drift
/// between machines is never mistaken for nonsense.
const CLOCK_SKEW_GRACE_SECS: i64 = 5 * 60;

/// Seconds since the epoch.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// A freshly minted invite. The token is returned **once**, here, and never stored in
/// a form it could be read back out of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Invite {
    pub token: String,
    /// When it stops working, so the page can say so rather than imply forever.
    pub expires_at: i64,
}

/// `POST /api/kitchens/{id}/invite` — mint an invite to this kitchen. Members only.
///
/// Minted per ask rather than kept on the kitchen. A standing token is a key under the
/// mat: it outlives the occasion it was shared for, and since everyone in a kitchen is
/// an owner of it, anyone who ever saw the link would keep full access forever.
pub async fn invite(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<Invite>, AppError> {
    let db = state.db()?;
    require_member(&db, &id, &user.telegram_user_id).await?;

    let token = mint(16);
    let expires_at = now() + INVITE_TTL_SECS;
    store_invite(&db, &token, &id, &user.telegram_user_id, expires_at)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(Invite { token, expires_at }))
}

/// `POST /api/kitchens/join` — join a kitchen by an invite, as a member like any other.
pub async fn join(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Json(body): Json<JoinBody>,
) -> Result<Json<KitchenDetail>, AppError> {
    let id = join_by_token(&state.db()?, body.token.trim(), &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest("no kitchen for that invite".into()))?;
    require_member(&state.db()?, &id, &user.telegram_user_id).await?;
    load_detail(&state.db()?, &id, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
        .map(Json)
}

/// `POST /api/kitchens/{id}/equipment` — add a piece of equipment.
/// A kitchen selects equipment from the corpus vocabulary and may not invent it (#81).
///
/// Checked here and not only in the picker, because a rule the client enforces is not
/// a rule: the endpoint takes whatever it is sent. And the reason is not tidiness —
/// the only use of knowing what a kitchen owns is matching it against recipes, so an
/// item no recipe asks for could never change what you are able to cook. It would sit
/// in the list making the kitchen look better equipped than it is.
///
/// Before the corpus has been read the vocabulary is empty and nothing can be added.
/// That is the ruling working, not a bug: there is genuinely nothing legitimate to
/// add yet.
pub async fn add_equipment(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<ItemBody>,
) -> Result<Json<KitchenDetail>, AppError> {
    let raw = body.item.trim();
    let item = crate::equipment::normalise_known(&state.db()?, raw)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "{raw:?} is not equipment any recipe asks for — pick from the list"
            ))
        })?;
    mutate_item(&state, &user, &id, "kitchen_equipment", &item, Op::Add).await
}

/// `DELETE /api/kitchens/{id}/equipment?item=…` — remove a piece of equipment.
pub async fn remove_equipment(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
    Query(q): Query<ItemQuery>,
) -> Result<Json<KitchenDetail>, AppError> {
    mutate_item(
        &state,
        &user,
        &id,
        "kitchen_equipment",
        q.item.trim(),
        Op::Remove,
    )
    .await
}

/// `POST /api/kitchens/{id}/pantry` — add an ingredient on hand.
pub async fn add_pantry(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<ItemBody>,
) -> Result<Json<KitchenDetail>, AppError> {
    mutate_item(
        &state,
        &user,
        &id,
        "kitchen_pantry",
        body.item.trim(),
        Op::Add,
    )
    .await
}

/// `DELETE /api/kitchens/{id}/pantry?item=…` — remove an ingredient on hand.
pub async fn remove_pantry(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
    Query(q): Query<ItemQuery>,
) -> Result<Json<KitchenDetail>, AppError> {
    mutate_item(
        &state,
        &user,
        &id,
        "kitchen_pantry",
        q.item.trim(),
        Op::Remove,
    )
    .await
}

enum Op {
    Add,
    Remove,
}

/// Shared body of the four inventory endpoints: require membership, then add/remove an
/// item and return the fresh detail. `table` is a fixed `&'static str` (never user
/// input), so interpolating it into the SQL is safe.
async fn mutate_item(
    state: &AppState,
    user: &CurrentUser,
    kitchen_id: &str,
    table: &'static str,
    item: &str,
    op: Op,
) -> Result<Json<KitchenDetail>, AppError> {
    require_member(&state.db()?, kitchen_id, &user.telegram_user_id).await?;
    if item.is_empty() {
        return Err(AppError::BadRequest("item is required".into()));
    }
    let res = match op {
        Op::Add => add_item(&state.db()?, table, kitchen_id, item).await,
        Op::Remove => remove_item(&state.db()?, table, kitchen_id, item).await,
    };
    res.map_err(|e| AppError::Internal(e.to_string()))?;
    load_detail(&state.db()?, kitchen_id, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
        .map(Json)
}

/// Whether this kitchen is the caller's primary, or `Forbidden` if they are not in it
/// — the gate every kitchen-scoped read/write passes through.
async fn require_member(conn: &Connection, kitchen_id: &str, user: &str) -> Result<bool, AppError> {
    membership(conn, kitchen_id, user)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::Forbidden("not a member of this kitchen".into()))
}

// --- persistence (pure, testable) ----------------------------------------------

/// Create a kitchen owned by `owner` and seat the owner as its first member. Returns
/// the new kitchen's id.
///
/// Both writes go in one transaction. A kitchen row without its membership row is a
/// kitchen nobody is in: `list_kitchens` joins `kitchen_members`, so it would show up
/// for no one and be reachable by no one.
async fn create_kitchen(conn: &Connection, name: &str, owner: &str) -> anyhow::Result<String> {
    create_owned(conn, name, owner, false).await
}

/// Create a kitchen owned by `owner`, optionally as their primary.
async fn create_owned(
    conn: &Connection,
    name: &str,
    owner: &str,
    primary: bool,
) -> anyhow::Result<String> {
    let id = mint(16);
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO kitchens (id, name, owner_id) VALUES (?1, ?2, ?3)",
        libsql::params![id.clone(), name.to_owned(), owner.to_owned()],
    )
    .await?;
    tx.execute(
        "INSERT INTO kitchen_members (kitchen_id, user_id, is_primary)
         VALUES (?1, ?2, ?3)",
        libsql::params![id.clone(), owner.to_owned(), i64::from(primary)],
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// The name a kitchen arrives with: the person's, when Telegram gave us one to use.
fn default_kitchen_name(username: Option<&str>) -> String {
    match username.map(str::trim) {
        Some(u) if !u.is_empty() => format!("{u}'s kitchen"),
        _ => "My kitchen".to_owned(),
    }
}

/// Whether `user` already has a primary kitchen.
async fn has_primary(conn: &Connection, user: &str) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM kitchen_members WHERE user_id = ?1 AND is_primary = 1",
            libsql::params![user.to_owned()],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// Make sure `user` has a primary kitchen, minting one named after them if not.
///
/// Two first visits can race: both read "no primary" and both insert. The unique index
/// on `(user_id) WHERE is_primary = 1` settles it — the loser's write is refused, and
/// a refusal here means somebody else did the job, so the invariant holds either way.
/// That is why the outcome is re-checked rather than the error inspected: what matters
/// is whether a primary exists now, not which call created it.
async fn ensure_primary(
    conn: &Connection,
    user: &str,
    username: Option<&str>,
) -> anyhow::Result<()> {
    if has_primary(conn, user).await? {
        return Ok(());
    }
    match create_owned(conn, &default_kitchen_name(username), user, true).await {
        Ok(_) => Ok(()),
        Err(e) => {
            if has_primary(conn, user).await? {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

/// Rename a kitchen.
async fn rename_kitchen(conn: &Connection, kitchen_id: &str, name: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE kitchens SET name = ?2 WHERE id = ?1",
        libsql::params![kitchen_id.to_owned(), name.to_owned()],
    )
    .await?;
    Ok(())
}

/// The caller's membership of a kitchen — whether it is their primary — or `None` if
/// they are not in it.
async fn membership(
    conn: &Connection,
    kitchen_id: &str,
    user: &str,
) -> anyhow::Result<Option<bool>> {
    let mut rows = conn
        .query(
            "SELECT is_primary FROM kitchen_members WHERE kitchen_id = ?1 AND user_id = ?2",
            libsql::params![kitchen_id.to_owned(), user.to_owned()],
        )
        .await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row.get::<i64>(0)? != 0)),
        None => Ok(None),
    }
}

/// The kitchens `user` is in: primary first, then oldest first.
async fn list_kitchens(conn: &Connection, user: &str) -> anyhow::Result<Vec<KitchenSummary>> {
    let mut rows = conn
        .query(
            "SELECT k.id, k.name, m.is_primary
             FROM kitchen_members m JOIN kitchens k ON k.id = m.kitchen_id
             WHERE m.user_id = ?1
             ORDER BY m.is_primary DESC, k.created_at, k.id",
            libsql::params![user.to_owned()],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(KitchenSummary {
            id: row.get::<String>(0)?,
            name: row.get::<String>(1)?,
            is_primary: row.get::<i64>(2)? != 0,
        });
    }
    Ok(out)
}

/// Mint an invite to `user`'s primary kitchen, for the Telegram bot (#25).
///
/// The bot is a person-facing surface like any other, so it goes through the same
/// guarantees rather than around them: the primary is made if it does not exist, and
/// the invite it returns is the same short-lived, hash-stored thing the web page gets.
/// Returns the kitchen's name alongside the token, so the reply can say what is being
/// handed out.
pub(crate) async fn primary_invite(
    conn: &Connection,
    user: &str,
    username: Option<&str>,
) -> anyhow::Result<Option<(String, String, i64)>> {
    ensure_primary(conn, user, username).await?;

    let mut rows = conn
        .query(
            "SELECT k.id, k.name
             FROM kitchen_members m JOIN kitchens k ON k.id = m.kitchen_id
             WHERE m.user_id = ?1 AND m.is_primary = 1",
            libsql::params![user.to_owned()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;

    let token = mint(16);
    let expires_at = now() + INVITE_TTL_SECS;
    store_invite(conn, &token, &id, user, expires_at).await?;
    Ok(Some((name, token, expires_at)))
}

/// Store an invite by its hash and both of its times, and clear out any that have died.
///
/// The token itself is never written down: the hash is the lookup key, exactly as for
/// sessions and login links (#25), so the database cannot hand anybody a working
/// invite. Sweeping here rather than on a timer keeps dead rows from accumulating
/// without anything having to run on a schedule — the table only grows while invites
/// are live.
///
/// The mint time is recorded as well as the expiry, because `now + 2h` trusts the
/// clock in one direction it should not: a clock running fast mints a link that
/// outlives the two hours it promised. Redemption refuses anything created in the
/// future, so once the clock is corrected such an invite is dead rather than
/// long-lived — and the sweep collects it, because an expiry that far out is one no
/// honest mint could have written and one that would otherwise never lapse. The grace
/// window keeps ordinary drift between machines from looking like that.
async fn store_invite(
    conn: &Connection,
    token: &str,
    kitchen_id: &str,
    created_by: &str,
    expires_at: i64,
) -> anyhow::Result<()> {
    // Two kinds of dead: expired, and impossible. A real invite expires at most
    // `INVITE_TTL_SECS` from the moment it was made, so one expiring beyond that came
    // from a clock that was wrong — and, crucially, its expiry is never reached, so
    // waiting for it to lapse would leave the row here forever. Both go.
    let at = now();
    conn.execute(
        "DELETE FROM kitchen_invites
         WHERE expires_at <= ?1 OR expires_at > ?2",
        libsql::params![at, at + INVITE_TTL_SECS + CLOCK_SKEW_GRACE_SECS],
    )
    .await?;
    conn.execute(
        "INSERT INTO kitchen_invites
             (token_hash, kitchen_id, created_by, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![
            crate::auth::hash_secret(token),
            kitchen_id.to_owned(),
            created_by.to_owned(),
            expires_at - INVITE_TTL_SECS,
            expires_at
        ],
    )
    .await?;
    Ok(())
}

/// Join a kitchen by an invite, as a member like any other. Idempotent: an existing member
/// keeps their place. Returns the kitchen id, or `None` if no kitchen has that
/// token.
async fn join_by_token(
    conn: &Connection,
    token: &str,
    user: &str,
) -> anyhow::Result<Option<String>> {
    let mut rows = conn
        .query(
            "SELECT kitchen_id FROM kitchen_invites
             WHERE token_hash = ?1 AND created_at <= ?2 AND expires_at > ?2",
            libsql::params![crate::auth::hash_secret(token), now()],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let id: String = row.get(0)?;
    conn.execute(
        "INSERT INTO kitchen_members (kitchen_id, user_id) VALUES (?1, ?2)
         ON CONFLICT(kitchen_id, user_id) DO NOTHING",
        libsql::params![id.clone(), user.to_owned()],
    )
    .await?;
    Ok(Some(id))
}

/// Add an item to an inventory table, idempotently (an add of what's there is a no-op).
async fn add_item(
    conn: &Connection,
    table: &'static str,
    kitchen_id: &str,
    item: &str,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO {table} (kitchen_id, item) VALUES (?1, ?2)
             ON CONFLICT(kitchen_id, item) DO NOTHING"
        ),
        libsql::params![kitchen_id.to_owned(), item.to_owned()],
    )
    .await?;
    Ok(())
}

/// Remove an item from an inventory table (a no-op if it isn't there).
async fn remove_item(
    conn: &Connection,
    table: &'static str,
    kitchen_id: &str,
    item: &str,
) -> anyhow::Result<()> {
    conn.execute(
        &format!("DELETE FROM {table} WHERE kitchen_id = ?1 AND item = ?2"),
        libsql::params![kitchen_id.to_owned(), item.to_owned()],
    )
    .await?;
    Ok(())
}

/// The full detail of a kitchen `caller` is a member of: its name + invite, every
/// member, and both inventories.
///
/// The caller's primary flag is read here rather than passed in, so it cannot disagree
/// with what the database says now.
async fn load_detail(
    conn: &Connection,
    kitchen_id: &str,
    caller: &str,
) -> anyhow::Result<KitchenDetail> {
    let is_primary = membership(conn, kitchen_id, caller)
        .await?
        .ok_or_else(|| anyhow::anyhow!("not a member of {kitchen_id}"))?;

    let mut rows = conn
        .query(
            "SELECT name FROM kitchens WHERE id = ?1",
            libsql::params![kitchen_id.to_owned()],
        )
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("kitchen vanished mid-request: {kitchen_id}"))?;
    let name: String = row.get(0)?;

    Ok(KitchenDetail {
        id: kitchen_id.to_owned(),
        name,
        is_primary,
        members: load_members(conn, kitchen_id).await?,
        equipment: load_items(conn, "kitchen_equipment", kitchen_id).await?,
        pantry: load_items(conn, "kitchen_pantry", kitchen_id).await?,
    })
}

async fn load_members(conn: &Connection, kitchen_id: &str) -> anyhow::Result<Vec<Member>> {
    let mut rows = conn
        .query(
            "SELECT m.user_id, u.username
             FROM kitchen_members m
             LEFT JOIN users u ON u.telegram_user_id = m.user_id
             WHERE m.kitchen_id = ?1
             ORDER BY m.joined_at, m.user_id",
            libsql::params![kitchen_id.to_owned()],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(Member {
            telegram_user_id: row.get::<String>(0)?,
            username: row.get::<Option<String>>(1)?,
        });
    }
    Ok(out)
}

async fn load_items(
    conn: &Connection,
    table: &'static str,
    kitchen_id: &str,
) -> anyhow::Result<Vec<String>> {
    let mut rows = conn
        .query(
            &format!("SELECT item FROM {table} WHERE kitchen_id = ?1 ORDER BY item"),
            libsql::params![kitchen_id.to_owned()],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(row.get::<String>(0)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn conn() -> Connection {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        conn
    }

    /// An invite works until it does not. This is the whole point of the change: a
    /// link that escapes — a screenshot, a forwarded message — stops being a key.
    #[tokio::test]
    async fn an_expired_invite_opens_nothing() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Home", "owner").await.unwrap();

        let live = mint(16);
        store_invite(&conn, &live, &id, "owner", now() + 60)
            .await
            .unwrap();
        assert_eq!(
            join_by_token(&conn, &live, "friend")
                .await
                .unwrap()
                .as_deref(),
            Some(id.as_str()),
            "a live invite seats you"
        );

        let dead = mint(16);
        store_invite(&conn, &dead, &id, "owner", now() - 1)
            .await
            .unwrap();
        assert_eq!(
            join_by_token(&conn, &dead, "stranger").await.unwrap(),
            None,
            "an expired invite is not a door"
        );
    }

    /// A clock running fast mints an invite that would outlive its two hours. Once the
    /// clock is right, that invite claims to come from the future — and is refused,
    /// rather than being honoured for however long the skew was.
    #[tokio::test]
    async fn an_invite_from_the_future_opens_nothing() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Home", "owner").await.unwrap();

        // What a clock an hour ahead would have written: minted "now + 1h", expiring
        // an hour beyond the honest two.
        let token = mint(16);
        store_invite(&conn, &token, &id, "owner", now() + 3600 + INVITE_TTL_SECS)
            .await
            .unwrap();

        assert_eq!(
            join_by_token(&conn, &token, "stranger").await.unwrap(),
            None,
            "an invite that has not been created yet is not a door"
        );
    }

    /// A far-future invite is not merely refused, it is collected. Its expiry never
    /// arrives, so leaving it to the ordinary sweep would leave it in the table for
    /// good — the exact opposite of what the sweep is for.
    #[tokio::test]
    async fn the_sweep_collects_impossible_invites() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Home", "owner").await.unwrap();

        // A year out: no honest mint could have written this.
        store_invite(&conn, &mint(16), &id, "owner", now() + 365 * 24 * 60 * 60)
            .await
            .unwrap();
        // The next mint sweeps it, and keeps its own.
        store_invite(&conn, &mint(16), &id, "owner", now() + INVITE_TTL_SECS)
            .await
            .unwrap();

        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM kitchen_invites WHERE kitchen_id = ?1",
                libsql::params![id.clone()],
            )
            .await
            .unwrap();
        let left: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(
            left, 1,
            "the impossible one was collected, the honest one kept"
        );
    }

    /// The token is not recoverable from the database — only its hash is stored, so a
    /// leaked backup is not a set of working links.
    #[tokio::test]
    async fn only_the_hash_is_stored() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Home", "owner").await.unwrap();
        let token = mint(16);
        store_invite(&conn, &token, &id, "owner", now() + 60)
            .await
            .unwrap();

        let mut rows = conn
            .query(
                "SELECT token_hash FROM kitchen_invites WHERE kitchen_id = ?1",
                libsql::params![id.clone()],
            )
            .await
            .unwrap();
        let stored: String = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_ne!(stored, token, "the token itself must never be written down");
        assert_eq!(stored, crate::auth::hash_secret(&token));
    }

    /// Minting sweeps what has died, so dead invites do not pile up unattended.
    #[tokio::test]
    async fn minting_clears_out_expired_invites() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Home", "owner").await.unwrap();
        store_invite(&conn, &mint(16), &id, "owner", now() - 1)
            .await
            .unwrap();
        store_invite(&conn, &mint(16), &id, "owner", now() + 60)
            .await
            .unwrap();

        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM kitchen_invites WHERE kitchen_id = ?1",
                libsql::params![id.clone()],
            )
            .await
            .unwrap();
        let live: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(live, 1, "the expired one was swept by the second mint");
    }

    /// Members come back with their usernames attached.
    ///
    /// This exists because it did not, and a column-index slip shipped: the query lost
    /// a column and the reads kept their old positions, so it asked for index 2 of a
    /// two-column row. Local SQLite tolerated that; the remote Hrana client unwraps an
    /// `Option` and panicked on every request in production. A test that only checks
    /// the *count* of members never touches the columns — so this one checks a value
    /// that has to be read out of the far end of the row.
    #[tokio::test]
    async fn members_carry_their_usernames() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO users (telegram_user_id, username) VALUES (?1, ?2)",
            libsql::params!["4242", "dave"],
        )
        .await
        .unwrap();
        let id = create_kitchen(&conn, "Home", "4242").await.unwrap();

        let members = load_members(&conn, &id).await.unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].telegram_user_id, "4242");
        assert_eq!(
            members[0].username.as_deref(),
            Some("dave"),
            "the username is read from the row, not merely present in the table"
        );
    }

    /// The bot's invite path reads two columns out of its own query and is otherwise
    /// only exercised through Telegram, which no test drives.
    #[tokio::test]
    async fn the_bot_invite_names_the_kitchen_it_opens() {
        let conn = conn().await;
        ensure_primary(&conn, "4242", Some("dave")).await.unwrap();

        let (kitchen, token, expires_at) = primary_invite(&conn, "4242", Some("dave"))
            .await
            .unwrap()
            .expect("a primary exists, so an invite can be made for it");

        assert_eq!(kitchen, "dave's kitchen", "the reply says what it opens");
        assert!(expires_at > now(), "and when it stops");

        // And it is a working invite, not merely a string.
        let id = list_kitchens(&conn, "4242").await.unwrap()[0].id.clone();
        assert_eq!(
            join_by_token(&conn, &token, "guest")
                .await
                .unwrap()
                .as_deref(),
            Some(id.as_str())
        );
    }

    /// Somebody with no kitchen at all gets one made before the invite is minted, so
    /// the bot never has to say "you have nowhere to invite anyone to".
    #[tokio::test]
    async fn the_bot_invite_makes_a_kitchen_if_you_have_none() {
        let conn = conn().await;
        assert!(list_kitchens(&conn, "9317").await.unwrap().is_empty());

        let minted = primary_invite(&conn, "9317", None).await.unwrap();
        assert!(minted.is_some(), "a kitchen is made rather than refused");
        assert_eq!(
            minted.unwrap().0,
            "My kitchen",
            "named for someone with no username"
        );
        assert_eq!(list_kitchens(&conn, "9317").await.unwrap().len(), 1);
    }

    /// A kitchen may only own what some recipe asks for (#81). Checked at the
    /// endpoint, because a rule only the picker enforces is not a rule — the endpoint
    /// takes whatever it is sent.
    #[tokio::test]
    async fn a_kitchen_can_only_own_what_recipes_ask_for() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO recipes (source, id, title, ingredients, instructions)
             VALUES ('themealdb', '1', 'T', '[]', 'Fry.')",
            (),
        )
        .await
        .unwrap();
        crate::equipment::submit(
            &conn,
            vec![crate::equipment::SubmittedEquipment {
                source: "themealdb".into(),
                id: "1".into(),
                equipment: vec![recipe_core::equipment::RequiredEquipment { item: "wok".into() }],
            }],
            "m",
        )
        .await
        .unwrap();

        // Known — and a typed capital is understood, because the strictness is about
        // which items exist, not about punishing spelling.
        assert_eq!(
            crate::equipment::normalise_known(&conn, "Wok")
                .await
                .unwrap(),
            Some("wok".to_string())
        );
        // Unknown — no recipe asks for it, so owning it could never change what you
        // are able to cook.
        assert_eq!(
            crate::equipment::normalise_known(&conn, "spurtle")
                .await
                .unwrap(),
            None
        );
    }

    /// Before the corpus has been read there is nothing legitimate to own. That is the
    /// ruling working, not a bug — and it is why the picker says so rather than
    /// offering an empty field.
    #[tokio::test]
    async fn an_unread_corpus_lets_a_kitchen_own_nothing() {
        let conn = conn().await;
        assert_eq!(
            crate::equipment::normalise_known(&conn, "wok")
                .await
                .unwrap(),
            None
        );
    }

    /// Nobody has zero kitchens: the first ask mints a primary named after them, and
    /// asking again does not mint a second.
    #[tokio::test]
    async fn ensure_primary_is_idempotent_and_named_after_you() {
        let conn = conn().await;

        ensure_primary(&conn, "u1", Some("dave")).await.unwrap();
        let first = list_kitchens(&conn, "u1").await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "dave's kitchen");
        assert!(first[0].is_primary);

        ensure_primary(&conn, "u1", Some("dave")).await.unwrap();
        let again = list_kitchens(&conn, "u1").await.unwrap();
        assert_eq!(again, first, "a second ask must not mint a second primary");
    }

    /// A renamed primary is still the primary — otherwise the next request would
    /// decide the user has none and mint another one alongside it.
    #[tokio::test]
    async fn renaming_the_primary_keeps_it_primary() {
        let conn = conn().await;
        ensure_primary(&conn, "u1", Some("dave")).await.unwrap();
        let id = list_kitchens(&conn, "u1").await.unwrap()[0].id.clone();

        rename_kitchen(&conn, &id, "The Shed").await.unwrap();
        ensure_primary(&conn, "u1", Some("dave")).await.unwrap();

        let after = list_kitchens(&conn, "u1").await.unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "The Shed");
        assert!(after[0].is_primary);
    }

    /// A Telegram account need not have a username, and the guarantee cannot depend on
    /// one — so the fallback name is used rather than a kitchen called "'s kitchen".
    #[test]
    fn default_name_survives_a_missing_username() {
        assert_eq!(default_kitchen_name(Some("dave")), "dave's kitchen");
        assert_eq!(default_kitchen_name(None), "My kitchen");
        assert_eq!(default_kitchen_name(Some("   ")), "My kitchen");
    }

    /// Kitchens you make yourself are ordinary ones — the primary stays the primary,
    /// and it leads the list so "the kitchen" is unambiguous.
    #[tokio::test]
    async fn a_made_kitchen_is_not_primary_and_the_primary_leads() {
        let conn = conn().await;
        ensure_primary(&conn, "u1", Some("dave")).await.unwrap();
        let extra = create_kitchen(&conn, "Beach house", "u1").await.unwrap();

        let list = list_kitchens(&conn, "u1").await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].is_primary, "the primary leads the list");
        assert_eq!(list[0].name, "dave's kitchen");
        assert_eq!(list[1].id, extra);
        assert!(!list[1].is_primary);
    }

    /// Being invited into someone else's kitchen does not move your primary, and does
    /// not make theirs yours.
    #[tokio::test]
    async fn joining_does_not_touch_your_primary() {
        let conn = conn().await;
        ensure_primary(&conn, "guest", Some("gina")).await.unwrap();
        let theirs = create_kitchen(&conn, "Their place", "owner").await.unwrap();
        let token = mint(16);
        store_invite(&conn, &token, &theirs, "owner", now() + 60)
            .await
            .unwrap();

        join_by_token(&conn, &token, "guest").await.unwrap();

        let list = list_kitchens(&conn, "guest").await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "gina's kitchen");
        assert!(list[0].is_primary, "your own kitchen stays your primary");
        assert_eq!(list[1].id, theirs);
        assert!(!list[1].is_primary);
    }

    /// A created kitchen seats its creator as owner, appears in their list, and its
    /// detail carries its members.
    #[tokio::test]
    async fn create_seats_owner_and_lists() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Home", "u1").await.unwrap();

        assert!(
            membership(&conn, &id, "u1").await.unwrap().is_some(),
            "in the kitchen"
        );
        assert_eq!(membership(&conn, &id, "u2").await.unwrap(), None);

        let list = list_kitchens(&conn, "u1").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Home");

        let detail = load_detail(&conn, &id, "u1").await.unwrap();
        assert_eq!(detail.members.len(), 1);
        assert_eq!(detail.members[0].telegram_user_id, "u1");
    }

    /// Joining by token seats you in the kitchen; a second join is a no-op;
    /// a bad token joins nothing.
    #[tokio::test]
    async fn join_by_token_is_idempotent_and_guarded() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Beach house", "owner").await.unwrap();
        let token = mint(16);
        store_invite(&conn, &token, &id, "owner", now() + 60)
            .await
            .unwrap();

        assert_eq!(
            join_by_token(&conn, &token, "guest1")
                .await
                .unwrap()
                .as_deref(),
            Some(id.as_str())
        );
        assert!(
            membership(&conn, &id, "guest1").await.unwrap().is_some(),
            "in the kitchen"
        );

        // Idempotent — a second join doesn't duplicate or demote.
        join_by_token(&conn, &token, "guest1").await.unwrap();
        let detail = load_detail(&conn, &id, "owner").await.unwrap();
        assert_eq!(
            detail.members.len(),
            2,
            "owner + one guest, not a duplicate"
        );

        // The owner re-redeeming their own link stays owner (DO NOTHING keeps the row).
        join_by_token(&conn, &token, "owner").await.unwrap();
        assert!(
            membership(&conn, &id, "owner").await.unwrap().is_some(),
            "in the kitchen"
        );

        // A bad token joins nothing.
        assert_eq!(join_by_token(&conn, "nope", "guest2").await.unwrap(), None);
        assert_eq!(membership(&conn, &id, "guest2").await.unwrap(), None);
    }

    /// Equipment and pantry adds are idempotent and independent; a remove clears one.
    #[tokio::test]
    async fn inventory_add_remove_is_idempotent_and_separate() {
        let conn = conn().await;
        let id = create_kitchen(&conn, "Home", "u1").await.unwrap();

        add_item(&conn, "kitchen_equipment", &id, "blender")
            .await
            .unwrap();
        add_item(&conn, "kitchen_equipment", &id, "blender")
            .await
            .unwrap(); // idempotent
        add_item(&conn, "kitchen_equipment", &id, "wok")
            .await
            .unwrap();
        add_item(&conn, "kitchen_pantry", &id, "rice")
            .await
            .unwrap();

        let detail = load_detail(&conn, &id, "u1").await.unwrap();
        assert_eq!(
            detail.equipment,
            vec!["blender".to_string(), "wok".to_string()]
        );
        assert_eq!(detail.pantry, vec!["rice".to_string()]);

        remove_item(&conn, "kitchen_equipment", &id, "wok")
            .await
            .unwrap();
        let detail = load_detail(&conn, &id, "u1").await.unwrap();
        assert_eq!(detail.equipment, vec!["blender".to_string()]);
        assert_eq!(detail.pantry, vec!["rice".to_string()], "pantry untouched");
    }

    /// A user sees only the kitchens they belong to.
    #[tokio::test]
    async fn list_is_scoped_to_membership() {
        let conn = conn().await;
        create_kitchen(&conn, "Mine", "u1").await.unwrap();
        create_kitchen(&conn, "Theirs", "u2").await.unwrap();
        assert_eq!(list_kitchens(&conn, "u1").await.unwrap().len(), 1);
        assert_eq!(list_kitchens(&conn, "u1").await.unwrap()[0].name, "Mine");
    }
}
