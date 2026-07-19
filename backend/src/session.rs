//! Cook-decider sessions (#20): the multiplayer mode of `pick`.
//!
//! A group opens a shared session and swipes yes/no through recipe cards; the app
//! tallies the yeses into "winners" — the group's decision. Everyone walks the
//! corpus **independently** (their own order), but a vote **cross-pollinates**:
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
    )
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(Created { channel_id }))
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

async fn session_exists(conn: &Connection, channel: &str) -> anyhow::Result<bool> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM decider_sessions WHERE channel_id = ?1",
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
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO decider_sessions (channel_id, created_by, filter) VALUES (?1, ?2, ?3)",
        libsql::params![channel_id, created_by, filter],
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
    #[tokio::test]
    async fn create_vote_and_tally() {
        let conn = conn().await;
        create_session(&conn, "chan1", "alice", None).await.unwrap();

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
        create_session(&conn, "c", "alice", None).await.unwrap();
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
        create_session(&conn, "c", "alice", None).await.unwrap();
        let (participants, rows) = load_tally(&conn, "c").await.unwrap();
        assert_eq!(participants, 0);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn session_existence_gates_join() {
        let conn = conn().await;
        assert!(!session_exists(&conn, "nope").await.unwrap());
        create_session(&conn, "yep", "alice", Some(r#"{"area":"Japanese"}"#))
            .await
            .unwrap();
        assert!(session_exists(&conn, "yep").await.unwrap());
    }
}
