//! Pick (#20): a live, shared swipe/vote over the corpus.
//!
//! People in a pick swipe yes/no through recipe cards; the app tallies the yeses
//! into "winners" — what to cook. Everyone walks the corpus **independently**
//! (their own order), but a vote **cross-pollinates**:
//! when anyone votes a recipe it is broadcast to the room, and every peer's client
//! silently slips that recipe into its own deck. So the group diverges for
//! discovery yet converges on every candidate anyone surfaced — which is what makes
//! the tally meaningful (everyone gets a shot at each voted recipe).
//!
//! **Turso is the source of truth; the WS room is only a live-push accelerator.**
//! Every vote is written to `votes` *and* broadcast over the room. A (re)joining
//! client — a late joiner, a 5-min-idle reconnect, or a reconnect after Render's
//! 15-min spin-down wiped every in-memory room — recovers the same way: read the
//! tally from Turso, then listen. A lost process is a performance blip, not data
//! loss.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use axum::{Extension, Json};
use futures_util::{SinkExt, StreamExt};
use libsql::Connection;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::auth::CurrentUser;
use crate::error::AppError;
use crate::AppState;

/// The live rooms: one broadcast channel per session, keyed by channel id, each
/// carrying JSON-serialized [`ServerMsg`] frames. Shared (cloned) in [`AppState`];
/// losing the map to a process restart is recovered from Turso on reconnect.
pub type Rooms = Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>;

/// A fresh, empty room registry for [`AppState`].
pub fn rooms() -> Rooms {
    Arc::new(Mutex::new(HashMap::new()))
}

/// The broadcast sender for `channel`, created on first join. The lock is held only
/// for the map lookup — never across an `.await`.
fn room(rooms: &Rooms, channel: &str) -> broadcast::Sender<String> {
    rooms
        .lock()
        .expect("rooms mutex poisoned")
        .entry(channel.to_string())
        .or_insert_with(|| broadcast::channel(256).0)
        .clone()
}

fn mint_channel_id() -> String {
    let mut buf = [0u8; 8];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

// ---- WS protocol -----------------------------------------------------------

/// A frame from a client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    /// This client's yes/no on a recipe. The voter is the authenticated session,
    /// never a field the client supplies — a client cannot vote as someone else.
    Vote {
        source: String,
        id: String,
        vote: bool,
    },
}

/// A frame to a client.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    /// The full tally, sent on join so a (re)connecting client rehydrates before
    /// listening. `participants` is the distinct-voter count — the client needs it
    /// to evaluate the consensus win condition (everyone said yes).
    Tally {
        participants: i64,
        votes: Vec<TallyRow>,
    },
    /// The lobby: how many people are deciding, and whether the swiping has begun.
    ///
    /// `deciders` is the roster count — the number a recipe has to win over. It comes
    /// from who *joined the plan*, not from who has voted or who happens to be
    /// connected: a person who steps away is still deciding, and a person who has not
    /// swiped yet has not agreed to anything.
    Lobby { deciders: i64, started: bool },
    /// One live vote — drives both the incremental tally and peer-injection (a
    /// client slips `source`/`id` into its own deck if it has not seen it).
    Vote {
        voter: String,
        source: String,
        id: String,
        vote: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
struct TallyRow {
    source: String,
    id: String,
    yes: i64,
    no: i64,
}

// ---- meal type -------------------------------------------------------------

/// Which meal a plan is for (#114): the strongest filter there is on what belongs
/// in the deck — nobody swipes pancakes and a lamb roast in the same session.
///
/// The full vocabulary is two tiers, and the tier is the type. This enum is the
/// **primary** tier — the meals you sit down to: breakfast, lunch, dinner, a
/// snack. The **secondary** tier ([`MealAddition`]: dessert, side, drink) is the
/// things that come *with* a meal. Splitting them into two types is what makes
/// the invalid states unrepresentable: a plan's meal type simply cannot be
/// "dessert" (it would claim the whole session for something that accompanies
/// it), and a chosen addition cannot be "dinner" — serde refuses both at the
/// wire, no handler checks anything.
///
/// A **fixed vocabulary**, not free text: unlike ingredients this is a small
/// closed set, so a picker over it can be exhaustive and stable, and the coming
/// meal-type reading of the corpus can share the same words (the union of both
/// tiers). Serde owns the wire form — always the lowercase name, and an unknown
/// or wrongly-cased value is rejected at deserialization, so no handler ever
/// holds a word outside its tier. The browser sentence-cases for display; the
/// wire and the database stay lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MealType {
    Breakfast,
    Lunch,
    Dinner,
    Snack,
}

/// A secondary choice on a plan (#114): something that comes *with* the meal —
/// dessert, a side, drinks — never the meal itself. See [`MealType`] for the
/// two-tier split; this is the tier a plan can carry **several** of, alongside
/// exactly one meal.
///
/// Chosen additions are recorded on the session and shown in the lobby, so the
/// room knows dinner comes with dessert. Whether a chosen addition one day gets
/// its **own pick round** (swipe the dinner, then swipe the dessert) is a real
/// possibility and deliberately not built here — this slice records and shows
/// the choice; the pick still runs one round, for the meal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MealAddition {
    Dessert,
    Side,
    Drink,
}

impl MealAddition {
    /// Every addition, in canonical order — the order stored, and the order the
    /// picker shows.
    pub const ALL: [MealAddition; 3] = [
        MealAddition::Dessert,
        MealAddition::Side,
        MealAddition::Drink,
    ];
}

/// The canonical form of a chosen-additions list: each addition at most once, in
/// vocabulary order. The list means a *set* — "dessert and drinks" — so a double
/// tap or a reordered client must not mint a different plan.
fn normalize_additions(input: &[MealAddition]) -> Vec<MealAddition> {
    MealAddition::ALL
        .iter()
        .copied()
        .filter(|a| input.contains(a))
        .collect()
}

/// A plan that names no meal is for dinner — the meal a group most plausibly
/// plans together. The same word migration 0016 backfills, so an unstated choice
/// and a pre-migration row read identically. Not time-of-day inference: the
/// default is one fixed word, and the host changes it in the lobby if it is wrong.
impl Default for MealType {
    fn default() -> Self {
        MealType::Dinner
    }
}

impl MealType {
    /// The lowercase canonical form — what the wire carries and the DB stores.
    fn as_str(self) -> &'static str {
        match self {
            MealType::Breakfast => "breakfast",
            MealType::Lunch => "lunch",
            MealType::Dinner => "dinner",
            MealType::Snack => "snack",
        }
    }

    /// The inverse of [`Self::as_str`], for reading a stored row back. `None` for
    /// anything outside the vocabulary — the caller decides how loud to be.
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "breakfast" => MealType::Breakfast,
            "lunch" => MealType::Lunch,
            "dinner" => MealType::Dinner,
            "snack" => MealType::Snack,
            _ => return None,
        })
    }
}

// ---- HTTP handlers ---------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    /// Optional JSON scope that seeds each participant's feed. Opaque here — the
    /// client interprets it; the backend only stores and echoes it.
    #[serde(default)]
    filter: Option<String>,
    /// The kitchen this plans a meal for. Optional so a plan can still be started
    /// outside one.
    #[serde(default)]
    kitchen_id: Option<String>,
    /// Which meal this plans (#114). Optional — an unstated choice is dinner
    /// ([`MealType::default`]) and the host can change it in the lobby.
    #[serde(default)]
    meal_type: Option<MealType>,
    /// What comes with it (#114) — dessert, a side, drinks. Optional; none is a
    /// plain meal, and the host can add them in the lobby.
    #[serde(default)]
    additions: Vec<MealAddition>,
}

#[derive(Debug, Serialize)]
pub struct Created {
    channel_id: String,
}

/// `POST /api/session` — start a session, returning its shareable channel id.
pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<CreateBody>,
) -> Result<Json<Created>, AppError> {
    // Minted once, *outside* the retryable closure, so every attempt writes the
    // same plan: a re-run after a lost response then hits the primary key and
    // fails honestly instead of minting a second plan (#130).
    let channel_id = mint_channel_id();
    // One closure — one connection per attempt — for both writes: the plan and
    // its first voter are one act, and two connections to a local database can
    // contend for the same write lock.
    {
        let channel_id = &channel_id;
        let user = &user;
        let body = &body;
        state.with_db(move |db| async move {
            create_session(
                &db,
                channel_id,
                &user.telegram_user_id,
                body.filter.as_deref(),
                body.kitchen_id.as_deref(),
                body.meal_type.unwrap_or_default(),
                &body.additions,
            )
            .await?;
            // The host is in their own plan from the moment it exists, so a lobby is
            // never empty and a plan never has nobody deciding it.
            seat_voter(&db, channel_id, &user.telegram_user_id).await
        })
    }
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(Created { channel_id }))
}

/// Who the host is pulling into the plan — a member of its kitchen, by their id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SeatBody {
    pub user_id: String,
}

/// A person in a plan. `username` is display convenience; identity is the id (#25).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Voter {
    pub telegram_user_id: String,
    pub username: Option<String>,
}

/// A plan's lobby: who is deciding, and whether it has begun.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LobbyView {
    pub channel_id: String,
    pub kitchen_id: Option<String>,
    /// Which meal this plans (#114) — what the room is deciding, so every voter
    /// sees it, and what the deck will one day be filtered by.
    pub meal_type: MealType,
    /// What comes with it (#114) — the chosen secondary things, each at most
    /// once, in vocabulary order. Recorded and shown; the pick itself still runs
    /// one round, for the meal.
    pub additions: Vec<MealAddition>,
    /// The telegram id that started it — only they can start the swiping.
    pub host: String,
    pub started: bool,
    pub voters: Vec<Voter>,
    /// Members of the plan's kitchen who are not yet deciding — the host can seat any
    /// of them without a link (#72). Empty when the plan has no kitchen, or once
    /// everyone in the kitchen is already in.
    pub candidates: Vec<Voter>,
}

/// `GET /api/session/{channel}` — the lobby: the roster, and whether it has started.
pub async fn lobby(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))
        .map(Json)
}

/// `POST /api/session/{channel}/join` — join a plan as a decider.
///
/// Only while the lobby is open. Once the swiping has begun the roster is what the
/// tally is measured against, so admitting someone late would move the target for
/// everyone already voting — every recipe that had won unanimously would silently
/// stop having done so.
pub async fn join_lobby(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let user = &user;
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    if view.started
        && !view
            .voters
            .iter()
            .any(|v| v.telegram_user_id == user.telegram_user_id)
    {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }
    state
        .with_db(move |db| async move { seat_voter(&db, channel, &user.telegram_user_id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// `POST /api/session/{channel}/start` — close the lobby and begin the pick. Host only.
pub async fn start(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can begin it".into(),
        ));
    }
    state
        .with_db(move |db| async move { begin_session(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// `POST /api/session/{channel}/seat` — the host pulls a kitchen member into the plan.
///
/// The counterpart to the invite link: the link is for people outside the kitchen, this
/// is for people already in it, who should not have to be sent anything. A meal is
/// planned *in* a kitchen (#72), so the seatable pool is exactly that kitchen's
/// members — seating an arbitrary id is refused.
///
/// Host only, and only before the swiping starts: the roster is what consensus is
/// measured against (#93), so it must not grow once people are voting.
///
/// A seated member is a decider whether or not they have opened the app yet — the
/// plan then waits on them, which is the point ("we're waiting on Mel"). If they are
/// not in fact cooking tonight, the host does not seat them, or they leave (#96).
pub async fn seat(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<SeatBody>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let body = &body;
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can add people to it".into(),
        ));
    }
    if view.started {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }
    let Some(kitchen_id) = view.kitchen_id.as_deref() else {
        return Err(AppError::BadRequest(
            "this plan is not in a kitchen, so it has no members to add — share the link".into(),
        ));
    };
    if !state
        .with_db(move |db| async move {
            crate::kitchens::is_member(&db, kitchen_id, &body.user_id).await
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    {
        return Err(AppError::BadRequest(
            "that person is not in this kitchen — share the link instead".into(),
        ));
    }

    state
        .with_db(move |db| async move { seat_voter(&db, channel, &body.user_id).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// The meal the host is declaring the plan to be for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct MealTypeBody {
    pub meal_type: MealType,
}

/// `POST /api/session/{channel}/meal-type` — the host names which meal this plans.
///
/// Every plan is born for dinner (the create default), so this is how the lobby
/// "picks one" (#114): the host flicks it to breakfast, lunch, a snack — and the
/// room announcement re-reads the lobby on every open client, so the whole roster
/// sees what it is deciding.
///
/// Host only, and only before the swiping starts — same shape as [`seat`], for the
/// same reason: once people are voting, the terms of the plan must not move under
/// them. A dinner nobody agreed on cannot retroactively become an agreed breakfast.
pub async fn set_meal_type(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<MealTypeBody>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can change which meal it is for".into(),
        ));
    }
    if view.started {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }

    state
        .with_db(move |db| async move { update_meal_type(&db, channel, body.meal_type).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// The secondary things the host is declaring alongside the meal. The whole
/// chosen set each time — a set, not a delta — so the picker's state and the
/// stored state cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AdditionsBody {
    pub additions: Vec<MealAddition>,
}

/// `POST /api/session/{channel}/additions` — the host names what comes with the
/// meal: dessert, a side, drinks.
///
/// Same guards as [`set_meal_type`], for the same reason — host only, and only
/// while the lobby is open; once people are voting, the terms of the plan must
/// not move under them. Announced to the room so every open client re-reads the
/// lobby.
pub async fn set_additions(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    Json(body): Json<AdditionsBody>,
) -> Result<Json<LobbyView>, AppError> {
    let channel = channel.as_str();
    let body = &body;
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;

    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can change what comes with the meal".into(),
        ));
    }
    if view.started {
        return Err(AppError::BadRequest(
            "this meal plan has already started".into(),
        ));
    }

    state
        .with_db(move |db| async move { update_additions(&db, channel, &body.additions).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, channel).await?;
    Ok(Json(view))
}

/// Re-read the lobby and tell the room, so every open client moves together — a guest
/// arriving, or the host pressing start, lands on everyone's screen at once.
async fn reload_and_announce(state: &AppState, channel: &str) -> Result<LobbyView, AppError> {
    let view = state
        .with_db(move |db| async move { load_lobby(&db, channel).await })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    let tx = room(&state.rooms, channel);
    if let Ok(txt) = serde_json::to_string(&ServerMsg::Lobby {
        deciders: view.voters.len() as i64,
        started: view.started,
    }) {
        // No receivers is an error and also a non-event: nobody is listening yet.
        let _ = tx.send(txt);
    }
    Ok(view)
}

/// `GET /api/session/{channel}/ws` — join a session's live room.
///
/// Session-gated like every person-facing route (#25); the upgrade carries the
/// session cookie, so the socket knows who is voting.
pub async fn ws(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, AppError> {
    // An unknown channel is a client bug, not a new room to conjure.
    if !{
        let channel = channel.as_str();
        state.with_db(move |db| async move { session_exists(&db, channel).await })
    }
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    {
        return Err(AppError::BadRequest(format!("unknown session: {channel}")));
    }
    let tx = room(&state.rooms, &channel);
    Ok(upgrade
        .on_upgrade(move |socket| socket_loop(socket, state, user.telegram_user_id, channel, tx)))
}

/// One connected client: rehydrate, then fan votes both ways until it drops.
async fn socket_loop(
    socket: WebSocket,
    state: AppState,
    voter: String,
    channel: String,
    tx: broadcast::Sender<String>,
) {
    let mut rx = tx.subscribe();
    let (mut sink, mut stream) = socket.split();

    // Not a handler: there is nowhere to return an error to, and this outlives a
    // request by design. It takes one connection for the life of the socket — if that
    // stream goes stale the client reconnects, which is the same recovery a dropped
    // socket already has.
    let Ok(db) = state.database.connect() else {
        return;
    };

    // Rehydrate: the current tally before any live vote.
    if let Ok((participants, votes)) = load_tally(&db, &channel).await {
        if let Ok(txt) = serde_json::to_string(&ServerMsg::Tally {
            participants,
            votes,
        }) {
            if sink.send(Message::Text(txt.into())).await.is_err() {
                return;
            }
        }
    }

    // The lobby, so a (re)connecting client knows how many it has to convince and
    // whether the swiping has begun, without a second round trip.
    if let Ok(Some(view)) = load_lobby(&db, &channel).await {
        if let Ok(txt) = serde_json::to_string(&ServerMsg::Lobby {
            deciders: view.voters.len() as i64,
            started: view.started,
        }) {
            if sink.send(Message::Text(txt.into())).await.is_err() {
                return;
            }
        }
    }

    // Render's free tier closes a WS idle for 5 min; a ping well inside that keeps
    // an active session's socket — and the box — awake.
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.tick().await; // the first tick fires immediately; consume it

    loop {
        tokio::select! {
            // A live vote from any peer (including this client's own echo) → forward.
            msg = rx.recv() => match msg {
                Ok(txt) => {
                    if sink.send(Message::Text(txt.into())).await.is_err() {
                        break;
                    }
                }
                // Fell behind the ring buffer; the client re-reads on its next
                // reconnect, so drop the gap rather than the connection.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            // A frame from this client.
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    if let Ok(ClientMsg::Vote { source, id, vote }) =
                        serde_json::from_str::<ClientMsg>(&t)
                    {
                        // Durable write first, then the live push — Turso is the truth.
                        if record_vote(&db, &channel, &source, &id, &voter, vote)
                            .await
                            .is_ok()
                        {
                            if let Ok(txt) = serde_json::to_string(&ServerMsg::Vote {
                                voter: voter.clone(),
                                source,
                                id,
                                vote,
                            }) {
                                // Err only means no receivers right now — harmless.
                                let _ = tx.send(txt);
                            }
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                // Ping/pong are handled by axum; other frames are ignored.
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
            _ = keepalive.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

// ---- persistence (pure, testable) ------------------------------------------

/// Seat someone in a plan. Idempotent — joining twice is one row, so a re-opened link
/// or a double tap does not inflate the number a recipe has to win over.
async fn seat_voter(conn: &Connection, channel: &str, user: &str) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pick_voters (channel_id, user_id) VALUES (?1, ?2)
         ON CONFLICT(channel_id, user_id) DO NOTHING",
        libsql::params![channel, user],
    )
    .await?;
    Ok(())
}

/// Close the lobby and begin the pick. Idempotent, and deliberately keeps the first
/// start time: pressing start twice must not move the moment the roster closed.
async fn begin_session(conn: &Connection, channel: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE pick_sessions SET started_at = unixepoch()
         WHERE channel_id = ?1 AND started_at IS NULL",
        libsql::params![channel],
    )
    .await?;
    Ok(())
}

/// A plan's lobby, or `None` if no such plan exists.
async fn load_lobby(conn: &Connection, channel: &str) -> anyhow::Result<Option<LobbyView>> {
    let mut rows = conn
        .query(
            "SELECT created_by, kitchen_id, started_at, meal_type, additions
             FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let host: String = row.get(0)?;
    let kitchen_id: Option<String> = row.get(1)?;
    let started_at: Option<i64> = row.get(2)?;
    // Every writer of these two columns validates against its tier's vocabulary,
    // so a stored word outside it is corruption — fail loud rather than serve a
    // plan for a meal that does not exist (the db.rs lesson: a wrong database
    // must not run beautifully).
    let meal_raw: String = row.get(3)?;
    let meal_type = MealType::parse(&meal_raw).ok_or_else(|| {
        anyhow::anyhow!("pick_sessions.meal_type outside the vocabulary: {meal_raw:?}")
    })?;
    let additions_raw: String = row.get(4)?;
    let additions: Vec<MealAddition> = serde_json::from_str(&additions_raw).map_err(|e| {
        anyhow::anyhow!("pick_sessions.additions outside the vocabulary: {additions_raw:?}: {e}")
    })?;

    let mut vrows = conn
        .query(
            "SELECT v.user_id, u.username
             FROM pick_voters v
             LEFT JOIN users u ON u.telegram_user_id = v.user_id
             WHERE v.channel_id = ?1
             ORDER BY v.joined_at, v.user_id",
            libsql::params![channel],
        )
        .await?;
    let mut voters = Vec::new();
    while let Some(v) = vrows.next().await? {
        voters.push(Voter {
            telegram_user_id: v.get::<String>(0)?,
            username: v.get::<Option<String>>(1)?,
        });
    }

    // The pool the host can pull in without a link: this kitchen's members who are not
    // already deciding. A plan with no kitchen has no such pool — it is invite-only.
    let candidates = match &kitchen_id {
        Some(kid) => {
            let seated: std::collections::HashSet<&str> =
                voters.iter().map(|v| v.telegram_user_id.as_str()).collect();
            crate::kitchens::member_list(conn, kid)
                .await?
                .into_iter()
                .filter(|(id, _)| !seated.contains(id.as_str()))
                .map(|(telegram_user_id, username)| Voter {
                    telegram_user_id,
                    username,
                })
                .collect()
        }
        None => Vec::new(),
    };

    Ok(Some(LobbyView {
        channel_id: channel.to_owned(),
        kitchen_id,
        meal_type,
        additions,
        host,
        started: started_at.is_some(),
        voters,
        candidates,
    }))
}

async fn session_exists(conn: &Connection, channel: &str) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// Insert a new session. `channel_id` is unique (the primary key).
pub async fn create_session(
    conn: &Connection,
    channel_id: &str,
    created_by: &str,
    filter: Option<&str>,
    kitchen_id: Option<&str>,
    meal_type: MealType,
    additions: &[MealAddition],
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pick_sessions (channel_id, created_by, filter, kitchen_id, meal_type, additions)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params![
            channel_id,
            created_by,
            filter,
            kitchen_id,
            meal_type.as_str(),
            serde_json::to_string(&normalize_additions(additions))?
        ],
    )
    .await?;
    Ok(())
}

/// Point a plan at a different meal (#114). The handler guards host + not-started;
/// this just writes the (already-validated, typed) word.
async fn update_meal_type(
    conn: &Connection,
    channel: &str,
    meal_type: MealType,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE pick_sessions SET meal_type = ?2 WHERE channel_id = ?1",
        libsql::params![channel, meal_type.as_str()],
    )
    .await?;
    Ok(())
}

/// Replace what comes with the meal (#114). The handler guards host + not-started;
/// this writes the (already-validated, typed) set in its canonical form.
async fn update_additions(
    conn: &Connection,
    channel: &str,
    additions: &[MealAddition],
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE pick_sessions SET additions = ?2 WHERE channel_id = ?1",
        libsql::params![
            channel,
            serde_json::to_string(&normalize_additions(additions))?
        ],
    )
    .await?;
    Ok(())
}

/// Record (or update) a voter's call on a recipe. Re-voting overwrites — a swipe is
/// a current decision, not an append.
async fn record_vote(
    conn: &Connection,
    channel: &str,
    source: &str,
    id: &str,
    voter: &str,
    vote: bool,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO votes (channel_id, source, id, voter_id, vote) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(channel_id, source, id, voter_id) DO UPDATE SET
            vote = excluded.vote,
            created_at = unixepoch()",
        libsql::params![channel, source, id, voter, vote as i64],
    )
    .await?;
    Ok(())
}

/// The tally for a channel: distinct-voter count plus per-recipe yes/no, ranked by
/// yeses. The client derives both win conditions from this — plurality (rank by
/// `yes`) and consensus (`yes == participants && no == 0`).
async fn load_tally(conn: &Connection, channel: &str) -> anyhow::Result<(i64, Vec<TallyRow>)> {
    let mut prows = conn
        .query(
            "SELECT COUNT(DISTINCT voter_id) FROM votes WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let participants: i64 = match prows.next().await? {
        Some(r) => r.get(0)?,
        None => 0,
    };

    let mut rows = conn
        .query(
            "SELECT source, id,
                    SUM(CASE WHEN vote = 1 THEN 1 ELSE 0 END) AS yes,
                    SUM(CASE WHEN vote = 0 THEN 1 ELSE 0 END) AS no
             FROM votes WHERE channel_id = ?1
             GROUP BY source, id
             ORDER BY yes DESC, no ASC",
            libsql::params![channel],
        )
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(TallyRow {
            source: r.get(0)?,
            id: r.get(1)?,
            yes: r.get(2)?,
            no: r.get(3)?,
        });
    }
    Ok((participants, out))
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

    fn row<'a>(rows: &'a [TallyRow], id: &str) -> &'a TallyRow {
        rows.iter().find(|r| r.id == id).expect("a tally row")
    }

    /// Two voters, two recipes: the tally counts yes/no per recipe and the distinct
    /// voters, and ranks by yeses — enough for the client to read both plurality and
    /// consensus off it.
    /// A plan is never roster-less and never double-counts: the host is seated once,
    /// however many times they arrive.
    #[tokio::test]
    async fn seating_is_idempotent_and_the_lobby_reads_back() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, Some("k1"), MealType::Dinner, &[])
            .await
            .unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        seat_voter(&conn, "c", "alice").await.unwrap();
        seat_voter(&conn, "c", "bob").await.unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.host, "alice");
        assert_eq!(view.kitchen_id.as_deref(), Some("k1"));
        assert!(!view.started, "a fresh plan is still in its lobby");
        assert_eq!(view.voters.len(), 2, "alice once, plus bob");
        assert_eq!(view.voters[0].telegram_user_id, "alice");
    }

    /// Voters come back with their usernames, which means reading the far column of
    /// the row rather than merely counting rows — the shape that hid a column-index
    /// slip in the kitchens lobby's twin query.
    #[tokio::test]
    async fn voters_carry_their_usernames() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO users (telegram_user_id, username) VALUES (?1, ?2)",
            libsql::params!["4242", "dave"],
        )
        .await
        .unwrap();
        create_session(&conn, "c", "4242", None, None, MealType::Dinner, &[])
            .await
            .unwrap();
        seat_voter(&conn, "c", "4242").await.unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.voters.len(), 1);
        assert_eq!(view.voters[0].telegram_user_id, "4242");
        assert_eq!(view.voters[0].username.as_deref(), Some("dave"));
    }

    /// A plan in a kitchen offers that kitchen's members as candidates, minus whoever
    /// is already deciding — the pool the host can pull in without a link (#72).
    #[tokio::test]
    async fn a_kitchen_plan_offers_its_members_as_candidates() {
        let conn = conn().await;
        let kid = crate::kitchens::create_kitchen(&conn, "Home", "host")
            .await
            .unwrap();
        crate::kitchens::seat_member_for_test(&conn, &kid, "mel").await;
        crate::kitchens::seat_member_for_test(&conn, &kid, "sam").await;

        create_session(&conn, "c", "host", None, Some(&kid), MealType::Dinner, &[])
            .await
            .unwrap();
        seat_voter(&conn, "c", "host").await.unwrap();

        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        let candidates: Vec<&str> = view
            .candidates
            .iter()
            .map(|v| v.telegram_user_id.as_str())
            .collect();
        assert!(candidates.contains(&"mel"), "{candidates:?}");
        assert!(candidates.contains(&"sam"), "{candidates:?}");
        assert!(
            !candidates.contains(&"host"),
            "the host is deciding, not a candidate"
        );
    }

    /// A plan with no kitchen has no candidate pool — it is invite-only.
    #[tokio::test]
    async fn a_kitchenless_plan_has_no_candidates() {
        let conn = conn().await;
        create_session(&conn, "c", "host", None, None, MealType::Dinner, &[])
            .await
            .unwrap();
        seat_voter(&conn, "c", "host").await.unwrap();
        assert!(load_lobby(&conn, "c")
            .await
            .unwrap()
            .unwrap()
            .candidates
            .is_empty());
    }

    /// Starting twice must not move the moment the roster closed — a second press is
    /// a no-op, not a re-start.
    #[tokio::test]
    async fn starting_is_idempotent() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[])
            .await
            .unwrap();
        begin_session(&conn, "c").await.unwrap();
        let first: Option<i64> = {
            let mut rows = conn
                .query(
                    "SELECT started_at FROM pick_sessions WHERE channel_id = ?1",
                    libsql::params!["c"],
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        };
        begin_session(&conn, "c").await.unwrap();
        let second: Option<i64> = {
            let mut rows = conn
                .query(
                    "SELECT started_at FROM pick_sessions WHERE channel_id = ?1",
                    libsql::params!["c"],
                )
                .await
                .unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        };
        assert!(first.is_some());
        assert_eq!(first, second, "the start time is the first one, always");
        assert!(load_lobby(&conn, "c").await.unwrap().unwrap().started);
    }

    /// A plan that does not exist has no lobby — and must not be conjured into one.
    #[tokio::test]
    async fn an_unknown_plan_has_no_lobby() {
        let conn = conn().await;
        assert!(load_lobby(&conn, "nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_vote_and_tally() {
        let conn = conn().await;
        create_session(&conn, "chan1", "alice", None, None, MealType::Dinner, &[])
            .await
            .unwrap();

        record_vote(&conn, "chan1", "themealdb", "r1", "alice", true)
            .await
            .unwrap();
        record_vote(&conn, "chan1", "themealdb", "r1", "bob", true)
            .await
            .unwrap();
        record_vote(&conn, "chan1", "themealdb", "r2", "alice", false)
            .await
            .unwrap();

        let (participants, rows) = load_tally(&conn, "chan1").await.unwrap();
        assert_eq!(participants, 2, "alice + bob");
        // r1 is the consensus/plurality winner: 2 yes, 0 no, == participants.
        assert_eq!((row(&rows, "r1").yes, row(&rows, "r1").no), (2, 0));
        assert_eq!((row(&rows, "r2").yes, row(&rows, "r2").no), (0, 1));
        assert_eq!(rows[0].id, "r1", "ranked by yeses");
    }

    /// A swipe is a current call, not an append: re-voting overwrites the row, so
    /// the tally never double-counts one person.
    #[tokio::test]
    async fn re_voting_updates_not_appends() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[])
            .await
            .unwrap();
        record_vote(&conn, "c", "s", "1", "alice", true)
            .await
            .unwrap();
        record_vote(&conn, "c", "s", "1", "alice", false)
            .await
            .unwrap();

        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(participants, 1);
        assert_eq!(rows.len(), 1, "one row, not two");
        assert_eq!((rows[0].yes, rows[0].no), (0, 1), "the changed-to no");
    }

    /// A channel with no votes yet tallies to nothing — the join rehydrate on a
    /// brand-new session is empty, not an error.
    #[tokio::test]
    async fn empty_channel_tallies_to_nothing() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[])
            .await
            .unwrap();
        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(participants, 0);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn session_existence_gates_join() {
        let conn = conn().await;
        assert!(!session_exists(&conn, "nope").await.unwrap());
        create_session(
            &conn,
            "yep",
            "alice",
            Some(r#"{"area":"Japanese"}"#),
            None,
            MealType::Dinner,
            &[],
        )
        .await
        .unwrap();
        assert!(session_exists(&conn, "yep").await.unwrap());
    }

    /// A plan is for a meal (#114): created for breakfast, its lobby says breakfast —
    /// stored and read back through the typed vocabulary, never a raw string.
    #[tokio::test]
    async fn a_plan_carries_its_meal_type() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Breakfast, &[])
            .await
            .unwrap();
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.meal_type, MealType::Breakfast);
    }

    /// An unstated choice is dinner, twice over and identically: the create handler
    /// fills `None` with [`MealType::default`], and migration 0016 backfills rows
    /// that predate the column with the same word — so a pre-#114 plan and a
    /// caller who named nothing read the same.
    #[tokio::test]
    async fn an_unstated_meal_type_is_dinner() {
        assert_eq!(MealType::default(), MealType::Dinner);

        // A body naming no meal deserializes, and resolves to the default —
        // exactly what the create handler does with it.
        let body: CreateBody = serde_json::from_str("{}").unwrap();
        assert_eq!(body.meal_type.unwrap_or_default(), MealType::Dinner);

        // A row written without the columns — the shape of every plan that existed
        // before migration 0016 — reads back as a plain dinner via the column
        // defaults: 'dinner', with nothing alongside.
        let conn = conn().await;
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by) VALUES ('old', 'alice')",
            (),
        )
        .await
        .unwrap();
        let view = load_lobby(&conn, "old").await.unwrap().unwrap();
        assert_eq!(view.meal_type, MealType::Dinner);
        assert!(view.additions.is_empty(), "a plain meal, nothing alongside");
    }

    /// The host repoints the plan at a different meal; the lobby follows.
    #[tokio::test]
    async fn the_meal_type_can_change_while_the_lobby_is_open() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[])
            .await
            .unwrap();
        update_meal_type(&conn, "c", MealType::Lunch).await.unwrap();
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(view.meal_type, MealType::Lunch);
    }

    /// The whole vocabulary survives the trip through storage: what `as_str` writes,
    /// `parse` reads, and the wire form serde emits is the same lowercase word — one
    /// canonical spelling everywhere, no second list to drift.
    #[test]
    fn the_meal_type_vocabulary_round_trips() {
        for t in [
            MealType::Breakfast,
            MealType::Lunch,
            MealType::Dinner,
            MealType::Snack,
        ] {
            assert_eq!(MealType::parse(t.as_str()), Some(t), "{t:?}");
            assert_eq!(
                serde_json::to_value(t).unwrap(),
                serde_json::Value::String(t.as_str().to_owned()),
                "the wire form and the stored form are the same word"
            );
        }
    }

    /// The two tiers partition the whole vocabulary: every word belongs to exactly
    /// one of [`MealType`] and [`MealAddition`], and each tier's serde refuses the
    /// other tier's words. This is the classification made checkable — "dessert is
    /// not a meal" is a type error, not a handler's opinion.
    #[test]
    fn the_two_tiers_partition_the_vocabulary() {
        let primary = ["breakfast", "lunch", "dinner", "snack"];
        let secondary = ["dessert", "side", "drink"];
        for word in primary {
            let q = format!("{word:?}");
            assert!(serde_json::from_str::<MealType>(&q).is_ok(), "{word}");
            assert!(
                serde_json::from_str::<MealAddition>(&q).is_err(),
                "{word} is a meal, not an addition"
            );
        }
        for word in secondary {
            let q = format!("{word:?}");
            assert!(serde_json::from_str::<MealAddition>(&q).is_ok(), "{word}");
            assert!(
                serde_json::from_str::<MealType>(&q).is_err(),
                "{word} is an addition to a meal, not a meal"
            );
        }
        assert_eq!(
            secondary.len(),
            MealAddition::ALL.len(),
            "ALL is the whole secondary tier"
        );
    }

    /// The vocabulary is closed at the wire: a word outside a field's tier — a
    /// made-up word, the right word in the wrong case, or the *other* tier's word —
    /// never reaches a handler, on create or on change. "dessert" as a meal type is
    /// the ruling made fixture: an addition *to* a meal is not a meal.
    #[test]
    fn a_word_outside_the_tier_is_rejected() {
        for bad in [
            r#"{"meal_type":"brunch"}"#,
            r#"{"meal_type":"Dinner"}"#,
            r#"{"meal_type":"dessert"}"#,
            r#"{"meal_type":"side"}"#,
            r#"{"meal_type":"drink"}"#,
            r#"{"additions":["dinner"]}"#,
            r#"{"additions":["Dessert"]}"#,
            r#"{"additions":["dessert","nonsense"]}"#,
        ] {
            assert!(serde_json::from_str::<CreateBody>(bad).is_err(), "{bad}");
        }
        for bad in [r#"{"meal_type":"brunch"}"#, r#"{"meal_type":"dessert"}"#] {
            assert!(serde_json::from_str::<MealTypeBody>(bad).is_err(), "{bad}");
        }
        for bad in [
            r#"{"additions":["dinner"]}"#,
            r#"{"additions":["dessert","nonsense"]}"#,
        ] {
            assert!(serde_json::from_str::<AdditionsBody>(bad).is_err(), "{bad}");
        }
    }

    /// A plan carries what comes with the meal: stored once each, in vocabulary
    /// order, however noisily the client said it — the list means a set.
    #[tokio::test]
    async fn additions_are_stored_deduped_in_vocabulary_order() {
        let conn = conn().await;
        create_session(
            &conn,
            "c",
            "alice",
            None,
            None,
            MealType::Dinner,
            &[
                MealAddition::Drink,
                MealAddition::Dessert,
                MealAddition::Drink,
            ],
        )
        .await
        .unwrap();
        let view = load_lobby(&conn, "c").await.unwrap().unwrap();
        assert_eq!(
            view.additions,
            vec![MealAddition::Dessert, MealAddition::Drink],
            "once each, dessert before drink"
        );
    }

    /// The host reshapes what comes with the meal while the lobby is open; the
    /// lobby follows, and clearing is just the empty set.
    #[tokio::test]
    async fn additions_can_change_while_the_lobby_is_open() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None, MealType::Dinner, &[])
            .await
            .unwrap();
        update_additions(&conn, "c", &[MealAddition::Side])
            .await
            .unwrap();
        assert_eq!(
            load_lobby(&conn, "c").await.unwrap().unwrap().additions,
            vec![MealAddition::Side]
        );
        update_additions(&conn, "c", &[]).await.unwrap();
        assert!(load_lobby(&conn, "c")
            .await
            .unwrap()
            .unwrap()
            .additions
            .is_empty());
    }

    /// A stored additions list outside the secondary tier is corruption — fail
    /// loud, exactly like a corrupt meal type.
    #[tokio::test]
    async fn corrupt_additions_fail_loud() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by, additions)
             VALUES ('bad', 'alice', '[\"dinner\"]')",
            (),
        )
        .await
        .unwrap();
        assert!(load_lobby(&conn, "bad").await.is_err());
    }

    /// Every writer validates, so a stored word outside the vocabulary is corruption —
    /// and the lobby fails loud rather than shrugging it into some default meal.
    #[tokio::test]
    async fn a_corrupt_meal_type_fails_loud() {
        let conn = conn().await;
        conn.execute(
            "INSERT INTO pick_sessions (channel_id, created_by, meal_type)
             VALUES ('bad', 'alice', 'brunch')",
            (),
        )
        .await
        .unwrap();
        assert!(load_lobby(&conn, "bad").await.is_err());
    }
}
