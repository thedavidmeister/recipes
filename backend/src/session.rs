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
    let channel_id = mint_channel_id();
    create_session(
        &state.db,
        &channel_id,
        &user.telegram_user_id,
        body.filter.as_deref(),
        body.kitchen_id.as_deref(),
    )
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    // The host is in their own plan from the moment it exists, so a lobby is never
    // empty and a plan never has nobody deciding it.
    seat_voter(&state.db, &channel_id, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(Created { channel_id }))
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
    /// The telegram id that started it — only they can start the swiping.
    pub host: String,
    pub started: bool,
    pub voters: Vec<Voter>,
}

/// `GET /api/session/{channel}` — the lobby: the roster, and whether it has started.
pub async fn lobby(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    load_lobby(&state.db, &channel)
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
    let view = load_lobby(&state.db, &channel)
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
    seat_voter(&state.db, &channel, &user.telegram_user_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, &channel).await?;
    Ok(Json(view))
}

/// `POST /api/session/{channel}/start` — close the lobby and begin the pick. Host only.
pub async fn start(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(channel): Path<String>,
) -> Result<Json<LobbyView>, AppError> {
    let view = load_lobby(&state.db, &channel)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest(format!("unknown session: {channel}")))?;
    if view.host != user.telegram_user_id {
        return Err(AppError::Forbidden(
            "only whoever started this plan can begin it".into(),
        ));
    }
    begin_session(&state.db, &channel)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let view = reload_and_announce(&state, &channel).await?;
    Ok(Json(view))
}

/// Re-read the lobby and tell the room, so every open client moves together — a guest
/// arriving, or the host pressing start, lands on everyone's screen at once.
async fn reload_and_announce(state: &AppState, channel: &str) -> Result<LobbyView, AppError> {
    let view = load_lobby(&state.db, channel)
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
    if !session_exists(&state.db, &channel)
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

    // Rehydrate: the current tally before any live vote.
    if let Ok((participants, votes)) = load_tally(&state.db, &channel).await {
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
    if let Ok(Some(view)) = load_lobby(&state.db, &channel).await {
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
                        if record_vote(&state.db, &channel, &source, &id, &voter, vote)
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
            "SELECT created_by, kitchen_id, started_at FROM pick_sessions WHERE channel_id = ?1",
            libsql::params![channel],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let host: String = row.get(0)?;
    let kitchen_id: Option<String> = row.get(1)?;
    let started_at: Option<i64> = row.get(2)?;

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

    Ok(Some(LobbyView {
        channel_id: channel.to_owned(),
        kitchen_id,
        host,
        started: started_at.is_some(),
        voters,
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
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO pick_sessions (channel_id, created_by, filter, kitchen_id)
         VALUES (?1, ?2, ?3, ?4)",
        libsql::params![channel_id, created_by, filter, kitchen_id],
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
        create_session(&conn, "c", "alice", None, Some("k1"))
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

    /// Starting twice must not move the moment the roster closed — a second press is
    /// a no-op, not a re-start.
    #[tokio::test]
    async fn starting_is_idempotent() {
        let conn = conn().await;
        create_session(&conn, "c", "alice", None, None)
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
        create_session(&conn, "chan1", "alice", None, None)
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
        create_session(&conn, "c", "alice", None, None)
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
        create_session(&conn, "c", "alice", None, None)
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
        create_session(&conn, "yep", "alice", Some(r#"{"area":"Japanese"}"#), None)
            .await
            .unwrap();
        assert!(session_exists(&conn, "yep").await.unwrap());
    }
}
