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

/// A kitchen in full — members, equipment, pantry, and the invite to share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KitchenDetail {
    pub id: String,
    pub name: String,
    /// Whether this is the **caller's** primary.
    pub is_primary: bool,
    pub invite_token: String,
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

/// `POST /api/kitchens/join` — join a kitchen by its invite token, as a guest.
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
pub async fn add_equipment(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<ItemBody>,
) -> Result<Json<KitchenDetail>, AppError> {
    mutate_item(
        &state,
        &user,
        &id,
        "kitchen_equipment",
        body.item.trim(),
        Op::Add,
    )
    .await
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
/// Both writes go in one transaction. A kitchen row without its owner's membership row
/// is a kitchen nobody is in — `list_kitchens` joins `kitchen_members`, so it shows up
/// for no one, while its `invite_token` is live and would seat a guest into a kitchen
/// with no owner.
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
    let token = mint(16);
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO kitchens (id, name, owner_id, invite_token) VALUES (?1, ?2, ?3, ?4)",
        libsql::params![id.clone(), name.to_owned(), owner.to_owned(), token],
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

/// Join a kitchen by its invite token, as a guest. Idempotent: an existing member
/// keeps their place. Returns the kitchen id, or `None` if no kitchen has that
/// token.
async fn join_by_token(
    conn: &Connection,
    token: &str,
    user: &str,
) -> anyhow::Result<Option<String>> {
    let mut rows = conn
        .query(
            "SELECT id FROM kitchens WHERE invite_token = ?1",
            libsql::params![token.to_owned()],
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
            "SELECT name, invite_token FROM kitchens WHERE id = ?1",
            libsql::params![kitchen_id.to_owned()],
        )
        .await?;
    let row = rows
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("kitchen vanished mid-request: {kitchen_id}"))?;
    let name: String = row.get(0)?;
    let invite_token: String = row.get(1)?;

    Ok(KitchenDetail {
        id: kitchen_id.to_owned(),
        name,
        is_primary,
        invite_token,
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

            username: row.get::<Option<String>>(2)?,
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
        let token: String = {
            let mut rows = conn
                .query(
                    "SELECT invite_token FROM kitchens WHERE id = ?1",
                    libsql::params![theirs.clone()],
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        };

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
        let token: String = {
            let mut rows = conn
                .query(
                    "SELECT invite_token FROM kitchens WHERE id = ?1",
                    libsql::params![id.clone()],
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        };

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
